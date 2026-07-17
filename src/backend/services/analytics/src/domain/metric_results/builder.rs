use std::collections::{BTreeMap, HashMap};

use toolkit_canonical_errors::CanonicalError;

use crate::domain::metric_definitions::{ComputationSpec, MetricDefinition};

use super::compiler::{
    BreakdownQueryRow, HistogramQueryRow, PeerQueryRow, PeriodQueryRow, TimeseriesQueryRow,
    UNKNOWN_DIMENSION_LABEL, UNKNOWN_DIMENSION_VALUE, dimension_aliases,
};
use super::dto::{
    BreakdownValueDto, ComputationDto, HistogramBinDto, HistogramValueDto, MetricDimensionDto,
    MetricResultDto, MetricResultViewDto, MetricResultsResponse, PeerValueDto, PeriodValueDto,
    TimeseriesDto, TimeseriesPointDto,
};
use super::validation::{
    HISTOGRAM_BINS, ValidatedMetricResultsRequest, enumerate_buckets, metric_result_too_large,
    row_limit,
};
use super::view::Bucket;

type DimensionKey = Vec<(String, String, Option<String>)>;
type SeriesKey = (String, DimensionKey);
type PointsByBucket = HashMap<String, Option<f64>>;

pub fn build_period_view(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    rows: Vec<PeriodQueryRow>,
) -> MetricResultViewDto {
    let values_by_entity: HashMap<String, Option<f64>> = rows
        .into_iter()
        .map(|row| (row.entity_id, row.value))
        .collect();
    let values = req
        .entity_ids
        .iter()
        .map(|entity_id| PeriodValueDto {
            entity_id: entity_id.clone(),
            value: values_by_entity
                .get(entity_id)
                .copied()
                .flatten()
                .or_else(|| {
                    if def.is_zero_filled() {
                        Some(zero_fill_value(def))
                    } else {
                        None
                    }
                }),
        })
        .collect();
    MetricResultViewDto::Period { values }
}

pub fn build_timeseries_view(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    bucket: Bucket,
    dimensions: &[String],
    rows: Vec<TimeseriesQueryRow>,
) -> Result<MetricResultViewDto, CanonicalError> {
    let buckets = enumerate_buckets(req.from, req.to, bucket);
    let mut by_series: BTreeMap<SeriesKey, PointsByBucket> = BTreeMap::new();

    if dimensions.is_empty() {
        for entity_id in &req.entity_ids {
            by_series
                .entry((entity_id.clone(), Vec::new()))
                .or_default();
        }
    }

    for row in rows {
        let dims = row_dimensions(&row.extra, dimensions)?;
        by_series
            .entry((row.entity_id, dims.clone()))
            .or_default()
            .insert(row.bucket_start, row.value);
    }

    let series = by_series
        .into_iter()
        .map(|((entity_id, dims), points_by_bucket)| {
            let points = buckets
                .iter()
                .map(|bucket| TimeseriesPointDto {
                    bucket_start: bucket.clone(),
                    value: points_by_bucket.get(bucket).copied().flatten().or_else(|| {
                        if def.is_zero_filled() {
                            Some(zero_fill_value(def))
                        } else {
                            None
                        }
                    }),
                })
                .collect();
            TimeseriesDto {
                entity_id,
                dimensions: dims
                    .into_iter()
                    .map(|(key, value, label)| MetricDimensionDto { key, value, label })
                    .collect(),
                points,
            }
        })
        .collect();

    Ok(MetricResultViewDto::Timeseries { bucket, series })
}

pub fn build_peer_view(rows: Vec<PeerQueryRow>) -> MetricResultViewDto {
    MetricResultViewDto::Peer {
        values: rows
            .into_iter()
            .map(|row| PeerValueDto {
                entity_id: row.entity_id,
                target_value: row.target_value,
                p25: row.p25,
                median: row.median,
                p75: row.p75,
                min: row.min,
                max: row.max,
                n: row.n.unwrap_or(0),
            })
            .collect(),
    }
}

pub fn build_breakdown_view(
    dimensions: &[String],
    rows: Vec<BreakdownQueryRow>,
) -> Result<MetricResultViewDto, CanonicalError> {
    let values = rows
        .into_iter()
        .map(|row| {
            Ok(BreakdownValueDto {
                entity_id: row.entity_id,
                dimensions: row_dimensions(&row.extra, dimensions)?
                    .into_iter()
                    .map(|(key, value, label)| MetricDimensionDto { key, value, label })
                    .collect(),
                value: row.value,
            })
        })
        .collect::<Result<Vec<_>, CanonicalError>>()?;
    Ok(MetricResultViewDto::Breakdown {
        dimensions: dimensions.iter().map(|d| (*d).clone()).collect(),
        values,
    })
}

/// Densifies histogram rows into the full fixed-bin shape. The SQL reports
/// only observed (entity, bin) pairs plus each entity's exact bounds; edge
/// math lives here alone so empty and observed bins can never disagree.
/// Every requested entity is listed; no events → empty `bins` (honest
/// absence, mirroring the period view's every-entity rule).
pub fn build_histogram_view(
    req: &ValidatedMetricResultsRequest,
    rows: Vec<HistogramQueryRow>,
) -> MetricResultViewDto {
    struct EntityBins {
        lo: f64,
        hi: f64,
        counts: HashMap<u32, u64>,
    }

    let mut by_entity: HashMap<String, EntityBins> = HashMap::new();
    for row in rows {
        let entry = by_entity.entry(row.entity_id).or_insert(EntityBins {
            lo: row.entity_lo,
            hi: row.entity_hi,
            counts: HashMap::new(),
        });
        let count = entry.counts.entry(row.bin_idx).or_insert(0);
        *count += row.bin_count.unwrap_or(0);
    }

    let bin_total = u32::try_from(HISTOGRAM_BINS).unwrap_or(u32::MAX);
    let values = req
        .entity_ids
        .iter()
        .map(|entity_id| {
            let bins = match by_entity.get(entity_id) {
                None => Vec::new(),
                // Bounds satisfy hi >= lo by construction; a collapsed range
                // (all values identical) renders as one [v, v] bin.
                Some(entity) if entity.hi <= entity.lo => vec![HistogramBinDto {
                    lo: entity.lo,
                    hi: entity.hi,
                    count: entity.counts.values().sum(),
                }],
                Some(entity) => {
                    let width = (entity.hi - entity.lo) / f64::from(bin_total);
                    (0..bin_total)
                        .map(|idx| HistogramBinDto {
                            lo: entity.lo + f64::from(idx) * width,
                            hi: if idx == bin_total - 1 {
                                entity.hi
                            } else {
                                entity.lo + f64::from(idx + 1) * width
                            },
                            count: entity.counts.get(&idx).copied().unwrap_or(0),
                        })
                        .collect()
                }
            };
            HistogramValueDto {
                entity_id: entity_id.clone(),
                bins,
            }
        })
        .collect();
    MetricResultViewDto::Histogram { values }
}

pub fn build_metric_result(
    def: &MetricDefinition,
    views: Vec<MetricResultViewDto>,
) -> MetricResultDto {
    let computation = match &def.spec {
        ComputationSpec::Sum { .. } => ComputationDto::Sum,
        ComputationSpec::Ratio { scale, .. } => ComputationDto::Ratio { scale: *scale },
        ComputationSpec::Median { .. } => ComputationDto::Median,
        ComputationSpec::DistinctCount { .. } => ComputationDto::DistinctCount,
    };
    MetricResultDto {
        metric_key: def.base.key.clone(),
        label: def.base.label.clone(),
        description: def.base.description.clone(),
        explanation: def.base.explanation.clone(),
        unit: def.base.unit.clone(),
        format: def.base.format,
        direction: def.base.direction,
        computation,
        views,
    }
}

// The fabricated zero for absent zero-filled entities, shaped by the
// definition's transform so a clamped or folded metric zero-fills to the
// value an actual all-zero aggregation would produce.
fn zero_fill_value(def: &MetricDefinition) -> f64 {
    match &def.transform {
        Some(transform) => transform.apply(0.0),
        None => 0.0,
    }
}

pub fn enforce_row_limit(response: &MetricResultsResponse) -> Result<(), CanonicalError> {
    if response_size(response) > row_limit() {
        return Err(metric_result_too_large());
    }
    Ok(())
}

fn response_size(response: &MetricResultsResponse) -> usize {
    response
        .metrics
        .iter()
        .flat_map(|metric| &metric.views)
        .map(|view| match view {
            MetricResultViewDto::Period { values } => values.len(),
            MetricResultViewDto::Timeseries { series, .. } => {
                series.iter().map(|s| s.points.len()).sum()
            }
            MetricResultViewDto::Peer { values } => values.len(),
            MetricResultViewDto::Breakdown { values, .. } => values.len(),
            MetricResultViewDto::Histogram { values } => {
                values.iter().map(|value| value.bins.len()).sum()
            }
        })
        .sum()
}

fn row_dimensions(
    extra: &HashMap<String, serde_json::Value>,
    dimensions: &[String],
) -> Result<Vec<(String, String, Option<String>)>, CanonicalError> {
    dimensions
        .iter()
        .enumerate()
        .map(|(idx, key)| {
            let (value_alias, label_alias) = dimension_aliases(idx);
            let value_field = extra.get(&value_alias).ok_or_else(|| {
                tracing::error!(alias = %value_alias, "metric result row missing dimension alias");
                CanonicalError::internal("metric result shape mismatch").create()
            })?;
            let label_field = extra.get(&label_alias).ok_or_else(|| {
                tracing::error!(alias = %label_alias, "metric result row missing dimension alias");
                CanonicalError::internal("metric result shape mismatch").create()
            })?;
            let value = json_string(Some(value_field))
                .unwrap_or_else(|| UNKNOWN_DIMENSION_VALUE.to_owned());
            let label =
                json_string(Some(label_field)).or_else(|| Some(UNKNOWN_DIMENSION_LABEL.to_owned()));
            Ok((key.clone(), value, label))
        })
        .collect()
}

fn json_string(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Null) | None => None,
        Some(v) => Some(v.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::definition::ValueTransform;
    use chrono::NaiveDate;
    use serde_json::json;

    use crate::domain::metric_definitions::definition::{
        MetricBase, MetricDirection, MetricFormat, MetricInput, MetricInputRole,
        ObservationRelation,
    };
    use crate::domain::metric_results::view::Bucket;

    fn base() -> MetricBase {
        MetricBase {
            key: "ai.accepted_lines".to_owned(),
            label: "AI-added lines".to_owned(),
            description: None,
            explanation: None,
            entity_type: "person".to_owned(),
            format: MetricFormat::Integer,
            unit: None,
            direction: MetricDirection::HigherIsBetter,
            peer_cohort_key: None,
            allowed_dimensions: vec!["tool".to_owned()],
        }
    }

    fn input(role: MetricInputRole, measure_key: &str) -> MetricInput {
        MetricInput {
            role,
            observation_relation: ObservationRelation::parse("ai_metric_observations")
                .unwrap_or_else(|| panic!("fixture relation must parse")),
            source_key: "ai_usage".to_owned(),
            measure_key: measure_key.to_owned(),
        }
    }

    fn sum_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(),
            spec: ComputationSpec::Sum {
                value: input(MetricInputRole::Value, "accepted_lines"),
            },
        }
    }

    fn ratio_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(),
            spec: ComputationSpec::Ratio {
                numerator: input(MetricInputRole::Numerator, "accepted_edit_actions"),
                denominator: input(MetricInputRole::Denominator, "tool_use_offered"),
                scale: 100.0,
            },
        }
    }

    fn median_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(),
            spec: ComputationSpec::Median {
                value: input(MetricInputRole::Value, "pr_cycle_hours"),
            },
        }
    }

    fn distinct_count_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(),
            spec: ComputationSpec::DistinctCount {
                value: input(MetricInputRole::Value, "active_day"),
            },
        }
    }

    fn histogram_row(
        entity_id: &str,
        bin_idx: u32,
        lo: f64,
        hi: f64,
        count: u64,
    ) -> HistogramQueryRow {
        HistogramQueryRow {
            entity_id: entity_id.to_owned(),
            bin_idx,
            entity_lo: lo,
            entity_hi: hi,
            bin_count: Some(count),
        }
    }

    fn request(entity_ids: Vec<&str>, from: &str, to: &str) -> ValidatedMetricResultsRequest {
        ValidatedMetricResultsRequest {
            entity_type: "person".to_owned(),
            entity_ids: entity_ids.into_iter().map(str::to_owned).collect(),
            from: match NaiveDate::parse_from_str(from, "%Y-%m-%d") {
                Ok(date) => date,
                Err(error) => panic!("bad test date {from}: {error}"),
            },
            to: match NaiveDate::parse_from_str(to, "%Y-%m-%d") {
                Ok(date) => date,
                Err(error) => panic!("bad test date {to}: {error}"),
            },
            metrics: Vec::new(),
        }
    }

    #[test]
    fn period_view_zero_fills_sum_and_keeps_request_order() {
        let req = request(vec!["b@x.io", "a@x.io"], "2026-01-01", "2026-01-31");
        let rows = vec![PeriodQueryRow {
            entity_id: "a@x.io".to_owned(),
            value: Some(5.0),
        }];
        let MetricResultViewDto::Period { values } = build_period_view(&sum_metric(), &req, rows)
        else {
            panic!("expected period view");
        };
        assert_eq!(values[0].entity_id, "b@x.io");
        assert_eq!(values[0].value, Some(0.0));
        assert_eq!(values[1].entity_id, "a@x.io");
        assert_eq!(values[1].value, Some(5.0));
    }

    #[test]
    fn period_view_keeps_ratio_nulls() {
        let req = request(vec!["a@x.io"], "2026-01-01", "2026-01-31");
        let MetricResultViewDto::Period { values } =
            build_period_view(&ratio_metric(), &req, Vec::new())
        else {
            panic!("expected period view");
        };
        assert_eq!(values[0].value, None);
    }

    #[test]
    fn period_view_keeps_median_nulls() {
        // A median of no events is unknowable, never zero.
        let req = request(vec!["a@x.io"], "2026-01-01", "2026-01-31");
        let MetricResultViewDto::Period { values } =
            build_period_view(&median_metric(), &req, Vec::new())
        else {
            panic!("expected period view");
        };
        assert_eq!(values[0].value, None);
    }

    #[test]
    fn period_view_zero_fills_distinct_count() {
        // Zero distinct subjects is a genuine zero, like a sum of nothing.
        let req = request(vec!["a@x.io"], "2026-01-01", "2026-01-31");
        let MetricResultViewDto::Period { values } =
            build_period_view(&distinct_count_metric(), &req, Vec::new())
        else {
            panic!("expected period view");
        };
        assert_eq!(values[0].value, Some(0.0));
    }

    #[test]
    fn histogram_view_densifies_fixed_bins_and_lists_every_entity() {
        let req = request(vec!["a@x.io", "b@x.io"], "2026-01-01", "2026-01-31");
        // a@x.io observed bins 0 and 9 over range [0, 100].
        let rows = vec![
            histogram_row("a@x.io", 0, 0.0, 100.0, 3),
            histogram_row("a@x.io", 9, 0.0, 100.0, 1),
        ];
        let MetricResultViewDto::Histogram { values } = build_histogram_view(&req, rows) else {
            panic!("expected histogram view");
        };
        assert_eq!(values.len(), 2);

        let a = &values[0];
        assert_eq!(a.entity_id, "a@x.io");
        assert_eq!(a.bins.len(), 10);
        assert_eq!(a.bins[0].count, 3);
        assert!((a.bins[0].lo - 0.0).abs() < f64::EPSILON);
        assert!((a.bins[0].hi - 10.0).abs() < f64::EPSILON);
        // Gap bins densify to zero with derived edges.
        assert_eq!(a.bins[4].count, 0);
        assert!((a.bins[4].lo - 40.0).abs() < f64::EPSILON);
        // Last bin closes exactly at the entity max.
        assert_eq!(a.bins[9].count, 1);
        assert!((a.bins[9].hi - 100.0).abs() < f64::EPSILON);

        // Entity with no events stays listed with honest empty bins.
        let b = &values[1];
        assert_eq!(b.entity_id, "b@x.io");
        assert!(b.bins.is_empty());
    }

    #[test]
    fn histogram_view_collapses_identical_values_to_single_bin() {
        let req = request(vec!["a@x.io"], "2026-01-01", "2026-01-31");
        let rows = vec![histogram_row("a@x.io", 0, 7.5, 7.5, 4)];
        let MetricResultViewDto::Histogram { values } = build_histogram_view(&req, rows) else {
            panic!("expected histogram view");
        };
        assert_eq!(values[0].bins.len(), 1);
        assert!((values[0].bins[0].lo - 7.5).abs() < f64::EPSILON);
        assert!((values[0].bins[0].hi - 7.5).abs() < f64::EPSILON);
        assert_eq!(values[0].bins[0].count, 4);
    }

    #[test]
    fn timeseries_densifies_all_buckets_per_entity() {
        let req = request(vec!["a@x.io"], "2026-01-01", "2026-01-03");
        let rows = vec![TimeseriesQueryRow {
            entity_id: "a@x.io".to_owned(),
            bucket_start: "2026-01-02".to_owned(),
            value: Some(3.0),
            extra: HashMap::new(),
        }];
        let Ok(MetricResultViewDto::Timeseries { series, .. }) =
            build_timeseries_view(&sum_metric(), &req, Bucket::Day, &[], rows)
        else {
            panic!("expected timeseries view");
        };
        assert_eq!(series.len(), 1);
        let points = &series[0].points;
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].value, Some(0.0));
        assert_eq!(points[1].value, Some(3.0));
        assert_eq!(points[2].value, Some(0.0));
    }

    #[test]
    fn ungrouped_timeseries_emits_series_for_entities_without_rows() {
        let req = request(vec!["a@x.io", "b@x.io"], "2026-01-01", "2026-01-02");
        let Ok(MetricResultViewDto::Timeseries { series, .. }) =
            build_timeseries_view(&ratio_metric(), &req, Bucket::Day, &[], Vec::new())
        else {
            panic!("expected timeseries view");
        };
        assert_eq!(series.len(), 2);
        assert!(series.iter().all(|s| s.points.len() == 2));
        assert!(
            series
                .iter()
                .all(|s| s.points.iter().all(|p| p.value.is_none()))
        );
    }

    #[test]
    fn dimensioned_timeseries_groups_by_observed_dimensions() {
        let req = request(vec!["a@x.io"], "2026-01-01", "2026-01-02");
        let mut extra = HashMap::new();
        extra.insert("dim_0_value".to_owned(), json!("cursor"));
        extra.insert("dim_0_label".to_owned(), json!("Cursor"));
        let rows = vec![TimeseriesQueryRow {
            entity_id: "a@x.io".to_owned(),
            bucket_start: "2026-01-01".to_owned(),
            value: Some(2.0),
            extra,
        }];
        let dimensions = vec!["tool".to_owned()];
        let Ok(MetricResultViewDto::Timeseries { series, .. }) =
            build_timeseries_view(&sum_metric(), &req, Bucket::Day, &dimensions, rows)
        else {
            panic!("expected timeseries view");
        };
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].dimensions[0].key, "tool");
        assert_eq!(series[0].dimensions[0].value, "cursor");
        assert_eq!(series[0].dimensions[0].label.as_deref(), Some("Cursor"));
        assert_eq!(series[0].points.len(), 2);
    }

    #[test]
    fn missing_dimension_alias_is_internal_error() {
        let req = request(vec!["a@x.io"], "2026-01-01", "2026-01-02");
        let rows = vec![TimeseriesQueryRow {
            entity_id: "a@x.io".to_owned(),
            bucket_start: "2026-01-01".to_owned(),
            value: Some(2.0),
            extra: HashMap::new(),
        }];
        let dimensions = vec!["tool".to_owned()];
        assert!(
            build_timeseries_view(&sum_metric(), &req, Bucket::Day, &dimensions, rows).is_err()
        );
    }

    #[test]
    fn breakdown_null_dimension_value_maps_to_unknown() {
        let mut extra = HashMap::new();
        extra.insert("dim_0_value".to_owned(), serde_json::Value::Null);
        extra.insert("dim_0_label".to_owned(), serde_json::Value::Null);
        let rows = vec![BreakdownQueryRow {
            entity_id: "a@x.io".to_owned(),
            value: Some(1.0),
            extra,
        }];
        let dimensions = vec!["tool".to_owned()];
        let Ok(MetricResultViewDto::Breakdown { values, .. }) =
            build_breakdown_view(&dimensions, rows)
        else {
            panic!("expected breakdown view");
        };
        assert_eq!(values[0].dimensions[0].value, UNKNOWN_DIMENSION_VALUE);
        assert_eq!(
            values[0].dimensions[0].label.as_deref(),
            Some(UNKNOWN_DIMENSION_LABEL)
        );
    }

    #[test]
    fn metric_result_wire_shape_is_flat_with_computation_tag() {
        let sum = build_metric_result(&sum_metric(), Vec::new());
        let sum_json = serde_json::to_value(&sum).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(sum_json["computation"], "sum");
        assert_eq!(sum_json["metric_key"], "ai.accepted_lines");
        assert_eq!(sum_json["format"], "integer");
        assert!(sum_json.get("scale").is_none());

        let ratio = build_metric_result(&ratio_metric(), Vec::new());
        let ratio_json = serde_json::to_value(&ratio).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ratio_json["computation"], "ratio");
        assert_eq!(ratio_json["scale"], 100.0);

        let median = build_metric_result(&median_metric(), Vec::new());
        let median_json = serde_json::to_value(&median).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(median_json["computation"], "median");
        assert!(median_json.get("scale").is_none());

        let distinct = build_metric_result(&distinct_count_metric(), Vec::new());
        let distinct_json = serde_json::to_value(&distinct).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(distinct_json["computation"], "distinct_count");
        assert!(distinct_json.get("scale").is_none());
    }

    #[test]
    fn histogram_wire_shape_uses_view_tag() {
        let req = request(vec!["a@x.io"], "2026-01-01", "2026-01-31");
        let view = build_histogram_view(&req, vec![histogram_row("a@x.io", 0, 1.0, 1.0, 2)]);
        let json = serde_json::to_value(&view).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(json["view"], "histogram");
        assert_eq!(json["values"][0]["entity_id"], "a@x.io");
        assert_eq!(json["values"][0]["bins"][0]["count"], 2);
        assert_eq!(json["values"][0]["bins"][0]["lo"], 1.0);
        assert_eq!(json["values"][0]["bins"][0]["hi"], 1.0);
    }

    #[test]
    fn response_size_counts_histogram_bins() {
        let req = request(vec!["a@x.io"], "2026-01-01", "2026-01-31");
        let view = build_histogram_view(&req, vec![histogram_row("a@x.io", 2, 0.0, 10.0, 1)]);
        let response = MetricResultsResponse {
            metrics: vec![build_metric_result(&median_metric(), vec![view])],
        };
        assert_eq!(response_size(&response), 10);
    }

    #[test]
    fn response_size_counts_densified_points() {
        let req = request(vec!["a@x.io"], "2026-01-01", "2026-01-10");
        let Ok(view) = build_timeseries_view(&sum_metric(), &req, Bucket::Day, &[], Vec::new())
        else {
            panic!("expected timeseries view");
        };
        let response = MetricResultsResponse {
            metrics: vec![build_metric_result(&sum_metric(), vec![view])],
        };
        assert_eq!(response_size(&response), 10);
    }

    #[test]
    fn zero_fill_applies_the_transform() {
        let mut def = sum_metric();
        def.transform = Some(ValueTransform {
            multiplier: Some(-1.0),
            offset: Some(100.0),
            clamp_min: Some(0.0),
            clamp_max: Some(100.0),
        });
        let req = request(vec!["absent@x.io"], "2026-01-01", "2026-01-31");
        let MetricResultViewDto::Period { values } = build_period_view(&def, &req, vec![]) else {
            panic!("expected period view");
        };
        assert_eq!(values[0].value, Some(100.0));
    }
}
