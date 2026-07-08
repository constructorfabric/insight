use std::collections::BTreeMap;
use std::collections::HashMap;

use serde::Deserialize;
use serde_json::json;
use toolkit_canonical_errors::CanonicalError;

use crate::domain::metric_definitions::MetricDefinition;

use super::compiler::{
    CompiledQuery, PeerQueryRow, PeriodQueryRow, compile_breakdown_query, compile_peer_batch_query,
    compile_period_batch_query, compile_timeseries_query,
};
use super::validation::{ValidatedMetricResultsRequest, ValidatedMetricView};
use super::view::Bucket;

#[derive(Debug)]
pub struct BatchItem {
    pub metric_index: usize,
    pub view_index: usize,
    pub def: MetricDefinition,
}

#[derive(Debug)]
pub enum UnbatchedView {
    Timeseries {
        bucket: Bucket,
        dimensions: Vec<String>,
    },
    Breakdown {
        dimensions: Vec<String>,
    },
}

#[derive(Debug)]
pub enum PlannedQuery {
    PeriodBatch {
        items: Vec<BatchItem>,
        query: CompiledQuery,
    },
    PeerBatch {
        items: Vec<BatchItem>,
        query: CompiledQuery,
    },
    Single {
        metric_index: usize,
        view_index: usize,
        def: Box<MetricDefinition>,
        view: UnbatchedView,
        query: CompiledQuery,
    },
}

pub fn plan_queries(req: &ValidatedMetricResultsRequest) -> Vec<PlannedQuery> {
    let mut period_groups: BTreeMap<&'static str, Vec<BatchItem>> = BTreeMap::new();
    let mut peer_groups: BTreeMap<(&'static str, String), Vec<BatchItem>> = BTreeMap::new();
    let mut singles = Vec::new();

    for (metric_index, metric) in req.metrics.iter().enumerate() {
        for (view_index, view) in metric.views.iter().enumerate() {
            let item = || BatchItem {
                metric_index,
                view_index,
                def: metric.def.clone(),
            };
            match view {
                ValidatedMetricView::Period => {
                    period_groups
                        .entry(metric.def.observation_source().source_ref())
                        .or_default()
                        .push(item());
                }
                ValidatedMetricView::Peer { cohort_key } => {
                    peer_groups
                        .entry((
                            metric.def.observation_source().source_ref(),
                            cohort_key.clone(),
                        ))
                        .or_default()
                        .push(item());
                }
                ValidatedMetricView::Timeseries { bucket, dimensions } => {
                    singles.push(PlannedQuery::Single {
                        metric_index,
                        view_index,
                        def: Box::new(metric.def.clone()),
                        view: UnbatchedView::Timeseries {
                            bucket: *bucket,
                            dimensions: dimensions.clone(),
                        },
                        query: compile_timeseries_query(&metric.def, req, *bucket, dimensions),
                    });
                }
                ValidatedMetricView::Breakdown { dimensions } => {
                    singles.push(PlannedQuery::Single {
                        metric_index,
                        view_index,
                        def: Box::new(metric.def.clone()),
                        view: UnbatchedView::Breakdown {
                            dimensions: dimensions.clone(),
                        },
                        query: compile_breakdown_query(&metric.def, req, dimensions),
                    });
                }
            }
        }
    }

    let mut planned = Vec::with_capacity(period_groups.len() + peer_groups.len() + singles.len());
    for items in period_groups.into_values() {
        let defs: Vec<&MetricDefinition> = items.iter().map(|item| &item.def).collect();
        let query = compile_period_batch_query(&defs, req);
        planned.push(PlannedQuery::PeriodBatch { items, query });
    }
    for ((_, cohort_key), items) in peer_groups {
        let defs: Vec<&MetricDefinition> = items.iter().map(|item| &item.def).collect();
        let query = compile_peer_batch_query(&defs, req, &cohort_key);
        planned.push(PlannedQuery::PeerBatch { items, query });
    }
    planned.extend(singles);
    planned
}

pub(crate) fn period_alias(item_index: usize) -> String {
    format!("m{item_index}")
}

pub(crate) struct PeerAliases {
    pub target: String,
    pub p25: String,
    pub median: String,
    pub p75: String,
    pub min: String,
    pub max: String,
    pub n: String,
}

pub(crate) fn peer_aliases(item_index: usize) -> PeerAliases {
    PeerAliases {
        target: format!("m{item_index}_target"),
        p25: format!("m{item_index}_p25"),
        median: format!("m{item_index}_median"),
        p75: format!("m{item_index}_p75"),
        min: format!("m{item_index}_min"),
        max: format!("m{item_index}_max"),
        n: format!("m{item_index}_n"),
    }
}

#[derive(Debug, Deserialize)]
pub struct PeriodWideRow {
    pub entity_id: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct PeerWideRow {
    pub entity_id: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

pub fn demux_period_rows(
    items: &[BatchItem],
    rows: Vec<PeriodWideRow>,
) -> Result<Vec<Vec<PeriodQueryRow>>, CanonicalError> {
    let mut per_item: Vec<Vec<PeriodQueryRow>> = items.iter().map(|_| Vec::new()).collect();
    for row in rows {
        for (item_index, item_rows) in per_item.iter_mut().enumerate() {
            let value = wide_field(&row.extra, &period_alias(item_index))?;
            let narrow = json!({ "entity_id": row.entity_id, "value": value });
            item_rows.push(decode_narrow_row(narrow)?);
        }
    }
    Ok(per_item)
}

pub fn demux_peer_rows(
    items: &[BatchItem],
    rows: Vec<PeerWideRow>,
) -> Result<Vec<Vec<PeerQueryRow>>, CanonicalError> {
    let mut per_item: Vec<Vec<PeerQueryRow>> = items.iter().map(|_| Vec::new()).collect();
    for row in rows {
        for (item_index, item_rows) in per_item.iter_mut().enumerate() {
            let aliases = peer_aliases(item_index);
            // Rebuilt as a narrow JSON row and decoded through PeerQueryRow so
            // its deserializers apply — ClickHouse quotes UInt64 in JSON output
            // (output_format_json_quote_64bit_integers), so `n` arrives as a
            // string and needs the optional_u64 path.
            let narrow = json!({
                "entity_id": row.entity_id,
                "target_value": wide_field(&row.extra, &aliases.target)?,
                "p25": wide_field(&row.extra, &aliases.p25)?,
                "median": wide_field(&row.extra, &aliases.median)?,
                "p75": wide_field(&row.extra, &aliases.p75)?,
                "min": wide_field(&row.extra, &aliases.min)?,
                "max": wide_field(&row.extra, &aliases.max)?,
                "n": wide_field(&row.extra, &aliases.n)?,
            });
            item_rows.push(decode_narrow_row(narrow)?);
        }
    }
    Ok(per_item)
}

fn wide_field<'a>(
    extra: &'a HashMap<String, serde_json::Value>,
    alias: &str,
) -> Result<&'a serde_json::Value, CanonicalError> {
    extra.get(alias).ok_or_else(|| {
        tracing::error!(alias = %alias, "batched metric result row missing item alias");
        CanonicalError::internal("metric result shape mismatch").create()
    })
}

fn decode_narrow_row<T: serde::de::DeserializeOwned>(
    narrow: serde_json::Value,
) -> Result<T, CanonicalError> {
    serde_json::from_value(narrow).map_err(|e| {
        tracing::error!(error = %e, "failed to decode demuxed metric result row");
        CanonicalError::internal("failed to parse query results").create()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serde_json::json;

    use crate::domain::metric_definitions::definition::{
        ComputationSpec, MetricBase, MetricDirection, MetricFormat, MetricInput, MetricInputRole,
        ObservationSource,
    };
    use crate::domain::metric_results::validation::ValidatedMetricRequest;

    fn def(key: &str, cohort_key: Option<&str>) -> MetricDefinition {
        MetricDefinition {
            base: MetricBase {
                key: key.to_owned(),
                label: key.to_owned(),
                description: None,
                explanation: None,
                entity_type: "person".to_owned(),
                format: MetricFormat::Integer,
                unit: None,
                direction: MetricDirection::HigherIsBetter,
                peer_cohort_key: cohort_key.map(str::to_owned),
                allowed_dimensions: vec!["tool".to_owned()],
            },
            spec: ComputationSpec::Sum {
                value: MetricInput {
                    role: MetricInputRole::Value,
                    observation_source: ObservationSource::AiMetricObservations,
                    source_key: "ai_usage".to_owned(),
                    measure_key: format!("{key}_measure"),
                },
            },
        }
    }

    fn request(metrics: Vec<ValidatedMetricRequest>) -> ValidatedMetricResultsRequest {
        ValidatedMetricResultsRequest {
            entity_type: "person".to_owned(),
            entity_ids: vec!["a@x.io".to_owned()],
            from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap_or_default(),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap_or_default(),
            metrics,
        }
    }

    fn views(views: Vec<ValidatedMetricView>, key: &str) -> ValidatedMetricRequest {
        ValidatedMetricRequest {
            def: def(key, Some("org_unit")),
            views,
        }
    }

    #[test]
    fn plan_groups_period_and_peer_views_into_batches() {
        let req = request(
            ["m_a", "m_b", "m_c"]
                .into_iter()
                .map(|key| {
                    views(
                        vec![
                            ValidatedMetricView::Period,
                            ValidatedMetricView::Peer {
                                cohort_key: "org_unit".to_owned(),
                            },
                            ValidatedMetricView::Timeseries {
                                bucket: Bucket::Day,
                                dimensions: vec![],
                            },
                        ],
                        key,
                    )
                })
                .collect(),
        );
        let planned = plan_queries(&req);
        assert_eq!(planned.len(), 5);
        let (mut period_batches, mut peer_batches, mut singles) = (0, 0, 0);
        for query in &planned {
            match query {
                PlannedQuery::PeriodBatch { items, .. } => {
                    period_batches += 1;
                    assert_eq!(
                        items
                            .iter()
                            .map(|i| (i.metric_index, i.view_index))
                            .collect::<Vec<_>>(),
                        vec![(0, 0), (1, 0), (2, 0)]
                    );
                }
                PlannedQuery::PeerBatch { items, .. } => {
                    peer_batches += 1;
                    assert_eq!(
                        items
                            .iter()
                            .map(|i| (i.metric_index, i.view_index))
                            .collect::<Vec<_>>(),
                        vec![(0, 1), (1, 1), (2, 1)]
                    );
                }
                PlannedQuery::Single {
                    metric_index,
                    view_index,
                    view: UnbatchedView::Timeseries { .. },
                    ..
                } => {
                    singles += 1;
                    assert_eq!(*view_index, 2);
                    assert!(*metric_index < 3);
                }
                PlannedQuery::Single { .. } => panic!("unexpected single view kind"),
            }
        }
        assert_eq!((period_batches, peer_batches, singles), (1, 1, 3));
    }

    #[test]
    fn plan_splits_peer_batches_by_cohort_key() {
        let req = request(vec![
            views(
                vec![ValidatedMetricView::Peer {
                    cohort_key: "org_unit".to_owned(),
                }],
                "m_a",
            ),
            views(
                vec![ValidatedMetricView::Peer {
                    cohort_key: "team".to_owned(),
                }],
                "m_b",
            ),
        ]);
        let planned = plan_queries(&req);
        assert_eq!(planned.len(), 2);
        assert!(
            planned
                .iter()
                .all(|q| matches!(q, PlannedQuery::PeerBatch { items, .. } if items.len() == 1))
        );
    }

    fn items(count: usize) -> Vec<BatchItem> {
        (0..count)
            .map(|i| BatchItem {
                metric_index: i,
                view_index: 0,
                def: def(&format!("m_{i}"), None),
            })
            .collect()
    }

    #[test]
    fn demux_period_maps_aliases_and_preserves_null() {
        let rows = vec![PeriodWideRow {
            entity_id: "a@x.io".to_owned(),
            extra: [
                ("m0".to_owned(), json!(1.5)),
                ("m1".to_owned(), json!(null)),
            ]
            .into_iter()
            .collect(),
        }];
        let Ok(per_item) = demux_period_rows(&items(2), rows) else {
            panic!("expected demux to succeed");
        };
        assert_eq!(per_item[0][0].value, Some(1.5));
        assert_eq!(per_item[1][0].value, None);
        assert!(per_item.iter().all(|rows| rows[0].entity_id == "a@x.io"));
    }

    #[test]
    fn demux_peer_parses_quoted_n() {
        // ClickHouse quotes UInt64 in JSONEachRow output by default; the
        // demuxed row must decode "7" through PeerQueryRow's optional_u64.
        let rows = vec![PeerWideRow {
            entity_id: "a@x.io".to_owned(),
            extra: [
                ("m0_target".to_owned(), json!(3.0)),
                ("m0_p25".to_owned(), json!(null)),
                ("m0_median".to_owned(), json!(null)),
                ("m0_p75".to_owned(), json!(null)),
                ("m0_min".to_owned(), json!(null)),
                ("m0_max".to_owned(), json!(null)),
                ("m0_n".to_owned(), json!("7")),
            ]
            .into_iter()
            .collect(),
        }];
        let Ok(per_item) = demux_peer_rows(&items(1), rows) else {
            panic!("expected demux to succeed");
        };
        let row = &per_item[0][0];
        assert_eq!(row.target_value, Some(3.0));
        assert_eq!(row.n, Some(7));
        assert_eq!(row.median, None);
    }

    #[test]
    fn demux_missing_alias_is_internal_error() {
        let period_rows = vec![PeriodWideRow {
            entity_id: "a@x.io".to_owned(),
            extra: HashMap::new(),
        }];
        assert!(demux_period_rows(&items(1), period_rows).is_err());

        let peer_rows = vec![PeerWideRow {
            entity_id: "a@x.io".to_owned(),
            extra: [("m0_target".to_owned(), json!(1.0))].into_iter().collect(),
        }];
        assert!(demux_peer_rows(&items(1), peer_rows).is_err());
    }
}
