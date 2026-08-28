use std::collections::{BTreeMap, HashMap};

use toolkit_canonical_errors::CanonicalError;

use crate::domain::external_links::ExternalSourceRegistry;
use crate::domain::metric_definitions::{ComputationSpec, MetricDefinition};

use super::batch::{RankedDimension, RankedGroup};
use super::compiler::{
    BreakdownQueryRow, HistogramQueryRow, PeerQueryRow, PeriodQueryRow, PooledHistogramQueryRow,
    RankingQueryRow, RollupQueryRow, TimeseriesQueryRow, UNKNOWN_DIMENSION_LABEL,
    UNKNOWN_DIMENSION_VALUE, dimension_aliases,
};
use super::dto::{
    BreakdownValueDto, ComputationDto, HistogramBinDto, HistogramValueDto, MetricDimensionDto,
    MetricResultDto, MetricResultViewDto, PeerValueDto, PeriodValueDto, RollupValueDto,
    TimeseriesDto, TimeseriesPointDto,
};
use super::validation::{
    HISTOGRAM_BINS, ValidatedMetricResultsRequest, enumerate_buckets, metric_result_too_large,
    row_limit,
};
use super::view::Bucket;

type DimensionKey = Vec<(String, String, Option<String>)>;
type SeriesKey = (String, bool, DimensionKey);
type PointsByBucket = HashMap<String, Option<f64>>;

struct SeriesData {
    points: PointsByBucket,
    total: Option<f64>,
    rank: Option<u32>,
    remainder: bool,
    label: Option<String>,
}

impl SeriesData {
    fn new(rank: Option<u32>, remainder: bool, label: Option<String>) -> Self {
        Self {
            points: HashMap::new(),
            total: None,
            rank,
            remainder,
            label,
        }
    }
}

pub fn build_period_view(
    _def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    rows: Vec<PeriodQueryRow>,
) -> MetricResultViewDto {
    let values_by_entity: HashMap<String, Option<f64>> = rows
        .into_iter()
        .map(|row| (req.entity.canonicalize_entity_id(row.entity_id), row.value))
        .collect();
    // `entity_id` IS the canonical contract on the wire and in the relations;
    // the selection renders the ids in that form, one rule for every view
    // builder in this module.
    let values = req
        .entity
        .entity_ids()
        .into_iter()
        .map(|entity_id| {
            let value = values_by_entity.get(&entity_id).copied().flatten();
            PeriodValueDto { entity_id, value }
        })
        .collect();
    MetricResultViewDto::Period { values }
}

pub fn build_timeseries_view(
    _def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    bucket: Bucket,
    dimensions: &[String],
    rows: Vec<TimeseriesQueryRow>,
) -> Result<MetricResultViewDto, CanonicalError> {
    let buckets = enumerate_buckets(req.from, req.to, bucket);
    let mut by_series: BTreeMap<SeriesKey, SeriesData> = BTreeMap::new();

    if dimensions.is_empty() {
        for entity_id in req.entity.entity_ids() {
            by_series
                .entry((entity_id, false, Vec::new()))
                .or_insert_with(|| SeriesData::new(None, false, None));
        }
    }

    for row in rows {
        let remainder = row.remainder != 0;
        let dims = if remainder {
            Vec::new()
        } else {
            row_dimensions(&row.extra, dimensions)?
        };
        let entity_id = req.entity.canonicalize_entity_id(row.entity_id);
        let data = by_series
            .entry((entity_id, remainder, dims))
            .or_insert_with(|| SeriesData::new(row.rank, remainder, row.group_label.clone()));
        if row.is_total != 0 {
            data.total = row.value;
        } else {
            data.points.insert(row.bucket_start, row.value);
        }
    }

    let mut series = by_series
        .into_iter()
        .map(|((entity_id, _, dims), data)| {
            let points = buckets
                .iter()
                .map(|bucket| TimeseriesPointDto {
                    bucket_start: bucket.clone(),
                    value: data.points.get(bucket).copied().flatten(),
                })
                .collect();
            TimeseriesDto {
                entity_id,
                dimensions: dims
                    .into_iter()
                    .map(|(key, value, label)| MetricDimensionDto {
                        key,
                        value,
                        label,
                        href: None,
                    })
                    .collect(),
                total: data.total,
                rank: data.rank,
                remainder: data.remainder.then_some(true),
                label: data.label,
                points,
            }
        })
        .collect::<Vec<_>>();
    series.sort_by(|left, right| {
        left.entity_id
            .cmp(&right.entity_id)
            .then_with(|| left.remainder.cmp(&right.remainder))
            .then_with(|| left.rank.cmp(&right.rank))
            .then_with(|| {
                left.dimensions
                    .iter()
                    .map(|dimension| &dimension.value)
                    .cmp(right.dimensions.iter().map(|dimension| &dimension.value))
            })
    });

    Ok(MetricResultViewDto::Timeseries { bucket, series })
}

pub fn build_ranked_groups(
    dimensions: &[String],
    rows: Vec<RankingQueryRow>,
) -> Result<Vec<RankedGroup>, CanonicalError> {
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            let rank = u32::try_from(index + 1)
                .map_err(|_| CanonicalError::internal("ranking result overflow").create())?;
            let dimensions = row_dimensions(&row.extra, dimensions)?
                .into_iter()
                .map(|(_, value, label)| RankedDimension { value, label })
                .collect();
            Ok(RankedGroup { rank, dimensions })
        })
        .collect()
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
    req: &ValidatedMetricResultsRequest,
    dimensions: &[String],
    rows: Vec<BreakdownQueryRow>,
    external_links: &ExternalSourceRegistry,
) -> Result<MetricResultViewDto, CanonicalError> {
    let values = rows
        .into_iter()
        .map(|row| {
            let mut dimensions = row_dimensions(&row.extra, dimensions)?
                .into_iter()
                .map(|(key, value, label)| MetricDimensionDto {
                    key,
                    value,
                    label,
                    href: None,
                })
                .collect::<Vec<_>>();
            if let Some(repository) = dimensions
                .iter_mut()
                .find(|dimension| dimension.key == "repository")
            {
                repository.href = external_links.repository_href(
                    row.source_provider.as_deref(),
                    row.source_id.as_deref(),
                    repository.label.as_deref(),
                );
            }
            Ok(BreakdownValueDto {
                entity_id: req.entity.canonicalize_entity_id(row.entity_id),
                dimensions,
                value: row.value,
            })
        })
        .collect::<Result<Vec<_>, CanonicalError>>()?;
    Ok(MetricResultViewDto::Breakdown {
        dimensions: dimensions.iter().map(|d| (*d).clone()).collect(),
        values,
    })
}

pub fn build_rollup_view(
    dimensions: &[String],
    rows: Vec<RollupQueryRow>,
) -> Result<MetricResultViewDto, CanonicalError> {
    let values = rows
        .into_iter()
        .map(|row| {
            let remainder = row.remainder != 0;
            let dimensions = if remainder {
                Vec::new()
            } else {
                row_dimensions(&row.extra, dimensions)?
                    .into_iter()
                    .map(|(key, value, label)| MetricDimensionDto {
                        key,
                        value,
                        label,
                        href: None,
                    })
                    .collect()
            };
            Ok(RollupValueDto {
                dimensions,
                value: row.value,
                contributing_entity_count: row.contributing_entity_count.unwrap_or(0),
                rank: row.rank,
                remainder: remainder.then_some(true),
                label: row.group_label,
            })
        })
        .collect::<Result<Vec<_>, CanonicalError>>()?;
    Ok(MetricResultViewDto::Rollup {
        dimensions: dimensions.to_vec(),
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
    let mut by_entity: HashMap<String, GroupBins> = HashMap::new();
    for row in rows {
        let entity_id = req.entity.canonicalize_entity_id(row.entity_id);
        by_entity
            .entry(entity_id)
            .or_insert_with(|| GroupBins::new(row.entity_lo, row.entity_hi))
            .add(row.bin_idx, row.bin_count.unwrap_or(0));
    }

    let values = req
        .entity
        .entity_ids()
        .into_iter()
        .map(|person_id| HistogramValueDto {
            bins: by_entity
                .get(&person_id)
                .map_or_else(Vec::new, GroupBins::densify),
            entity_id: Some(person_id),
            dimensions: Vec::new(),
        })
        .collect();
    MetricResultViewDto::Histogram {
        dimensions: Vec::new(),
        values,
    }
}

/// The pooled counterpart: one row per observed dimension tuple, binned over
/// every selected entity's events together. Unlike the per-entity shape there
/// is no roster to list against, so a tuple with no events simply has no row —
/// the same absence rule rollup follows.
pub fn build_pooled_histogram_view(
    rows: Vec<PooledHistogramQueryRow>,
    dimensions: &[String],
) -> Result<MetricResultViewDto, CanonicalError> {
    let mut by_group: Vec<(Vec<MetricDimensionDto>, GroupBins)> = Vec::new();
    let mut index: HashMap<Vec<String>, usize> = HashMap::new();
    for row in rows {
        let dims = row_dimensions(&row.extra, dimensions)?;
        let key: Vec<String> = dims.iter().map(|(_, value, _)| value.clone()).collect();
        let position = if let Some(position) = index.get(&key) {
            *position
        } else {
            by_group.push((
                dims.into_iter()
                    .map(|(key, value, label)| MetricDimensionDto {
                        key,
                        value,
                        label,
                        href: None,
                    })
                    .collect(),
                GroupBins::new(row.group_lo, row.group_hi),
            ));
            index.insert(key, by_group.len() - 1);
            by_group.len() - 1
        };
        by_group[position]
            .1
            .add(row.bin_idx, row.bin_count.unwrap_or(0));
    }

    let values = by_group
        .into_iter()
        .map(|(dims, bins)| HistogramValueDto {
            entity_id: None,
            dimensions: dims,
            bins: bins.densify(),
        })
        .collect();
    Ok(MetricResultViewDto::Histogram {
        dimensions: dimensions.to_vec(),
        values,
    })
}

/// One partition's exact value bounds plus its observed bin counts. The SQL
/// reports only observed (partition, bin) pairs, so the edge math that turns
/// them into the full fixed-bin shape lives here alone — empty and observed
/// bins can never disagree about a boundary.
struct GroupBins {
    lo: f64,
    hi: f64,
    counts: HashMap<u32, u64>,
}

impl GroupBins {
    fn new(lo: f64, hi: f64) -> Self {
        Self {
            lo,
            hi,
            counts: HashMap::new(),
        }
    }

    fn add(&mut self, bin_idx: u32, count: u64) {
        *self.counts.entry(bin_idx).or_insert(0) += count;
    }

    fn densify(&self) -> Vec<HistogramBinDto> {
        let bin_total = u32::try_from(HISTOGRAM_BINS).unwrap_or(u32::MAX);
        // Bounds satisfy hi >= lo by construction; a collapsed range (all
        // values identical) renders as one [v, v] bin.
        if self.hi <= self.lo {
            return vec![HistogramBinDto {
                lo: self.lo,
                hi: self.hi,
                count: self.counts.values().sum(),
            }];
        }
        let width = (self.hi - self.lo) / f64::from(bin_total);
        (0..bin_total)
            .map(|idx| HistogramBinDto {
                lo: self.lo + f64::from(idx) * width,
                hi: if idx == bin_total - 1 {
                    self.hi
                } else {
                    self.lo + f64::from(idx + 1) * width
                },
                count: self.counts.get(&idx).copied().unwrap_or(0),
            })
            .collect()
    }
}

pub fn build_metric_result(
    def: &MetricDefinition,
    views: Vec<MetricResultViewDto>,
    selection: super::dto::MetricResultSelectionDto,
) -> MetricResultDto {
    let computation = match &def.spec {
        ComputationSpec::Sum { .. } => ComputationDto::Sum,
        ComputationSpec::Ratio { scale, .. } => ComputationDto::Ratio { scale: *scale },
        ComputationSpec::Median { .. } => ComputationDto::Median,
        ComputationSpec::Percentile { q, .. } => ComputationDto::Percentile { q: *q },
        ComputationSpec::Stddev { .. } => ComputationDto::Stddev,
        ComputationSpec::DistinctCount { .. } => ComputationDto::DistinctCount,
    };
    MetricResultDto {
        metric_key: def.base.key.clone(),
        label: def.base.label.clone(),
        short_label: def.base.short_label.clone(),
        description: def.base.description.clone(),
        explanation: def.base.explanation.clone(),
        unit: def.base.unit.clone(),
        format: def.base.format,
        direction: def.base.direction,
        computation,
        views,
        drilldown: None,
        selection,
    }
}

pub fn enforce_view_row_limit(
    view: &MetricResultViewDto,
    field: impl Into<String>,
) -> Result<(), CanonicalError> {
    if view_size(view) > row_limit() {
        return Err(metric_result_too_large(field));
    }
    Ok(())
}

fn view_size(view: &MetricResultViewDto) -> usize {
    match view {
        MetricResultViewDto::Period { values } => values.len(),
        MetricResultViewDto::Timeseries { series, .. } => {
            series.iter().map(|series| series.points.len() + 1).sum()
        }
        MetricResultViewDto::Peer { values } => values.len(),
        MetricResultViewDto::Breakdown { values, .. } => values.len(),
        MetricResultViewDto::Rollup { values, .. } => values.len(),
        MetricResultViewDto::Histogram { values, .. } => {
            values.iter().map(|value| value.bins.len()).sum()
        }
        MetricResultViewDto::Error { .. } => 0,
    }
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
    use crate::domain::metric_results::validation::ValidatedEntitySelection;
    use uuid::Uuid;

    use super::*;
    use crate::domain::metric_definitions::definition::{
        RatioDenominatorAggregation, ValueTransform,
    };
    use chrono::NaiveDate;
    use serde_json::json;

    use crate::domain::metric_definitions::definition::{
        AliasCollapse, MetricBase, MetricDirection, MetricFormat, MetricInput, MetricInputRole,
        ObservationRelation, ObservationSource,
    };
    use crate::domain::metric_results::view::Bucket;

    fn base() -> MetricBase {
        MetricBase {
            key: "ai.accepted_lines".to_owned(),
            label: "AI-added lines".to_owned(),
            short_label: None,
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
            observation: ObservationSource::Managed(
                ObservationRelation::parse("ai_metric_observations")
                    .unwrap_or_else(|| panic!("fixture relation must parse")),
            ),
            source_key: "ai_usage".to_owned(),
            measure_key: measure_key.to_owned(),
            alias_collapse: AliasCollapse::Sum,
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
                denominator_aggregation: RatioDenominatorAggregation::Sum,
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

    fn percentile_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(),
            spec: ComputationSpec::Percentile {
                value: input(MetricInputRole::Value, "pr_cycle_hours"),
                q: 0.75,
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

    fn request(person_ids: Vec<&str>, from: &str, to: &str) -> ValidatedMetricResultsRequest {
        ValidatedMetricResultsRequest {
            tenant_id: uuid::Uuid::from_u128(0x1967),
            entity: ValidatedEntitySelection::Person {
                ids: person_ids
                    .into_iter()
                    .map(|id| match Uuid::parse_str(id) {
                        Ok(person_id) => person_id,
                        Err(error) => panic!("bad test person id {id}: {error}"),
                    })
                    .collect(),
            },
            from: match NaiveDate::parse_from_str(from, "%Y-%m-%d") {
                Ok(date) => date,
                Err(error) => panic!("bad test date {from}: {error}"),
            },
            to: match NaiveDate::parse_from_str(to, "%Y-%m-%d") {
                Ok(date) => date,
                Err(error) => panic!("bad test date {to}: {error}"),
            },
            metrics: Vec::new(),
            enforce_tenant_scope: true,
        }
    }

    #[test]
    fn period_view_keeps_missing_sum_null_and_request_order() {
        let req = request(
            vec![
                "00000000-0000-0000-0000-00000000000b",
                "00000000-0000-0000-0000-00000000000a",
            ],
            "2026-01-01",
            "2026-01-31",
        );
        let rows = vec![PeriodQueryRow {
            entity_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
            value: Some(5.0),
        }];
        let MetricResultViewDto::Period { values } = build_period_view(&sum_metric(), &req, rows)
        else {
            panic!("expected period view");
        };
        assert_eq!(values[0].entity_id, "00000000-0000-0000-0000-00000000000b");
        assert_eq!(values[0].value, None);
        assert_eq!(values[1].entity_id, "00000000-0000-0000-0000-00000000000a");
        assert_eq!(values[1].value, Some(5.0));
    }

    #[test]
    fn tenant_period_view_exposes_the_session_tenant_not_the_storage_key() {
        let tenant_id = Uuid::from_u128(0x1967);
        let mut req = request(Vec::new(), "2026-01-01", "2026-01-31");
        req.entity = ValidatedEntitySelection::Tenant { id: tenant_id };
        let rows = vec![PeriodQueryRow {
            entity_id: "default".to_owned(),
            value: Some(5.0),
        }];

        let MetricResultViewDto::Period { values } = build_period_view(&sum_metric(), &req, rows)
        else {
            panic!("expected period view");
        };

        assert_eq!(values[0].entity_id, tenant_id.to_string());
        assert_eq!(values[0].value, Some(5.0));
    }

    #[test]
    fn period_view_keeps_ratio_nulls() {
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-31",
        );
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
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-31",
        );
        let MetricResultViewDto::Period { values } =
            build_period_view(&median_metric(), &req, Vec::new())
        else {
            panic!("expected period view");
        };
        assert_eq!(values[0].value, None);
    }

    #[test]
    fn period_view_keeps_missing_distinct_count_null() {
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-31",
        );
        let MetricResultViewDto::Period { values } =
            build_period_view(&distinct_count_metric(), &req, Vec::new())
        else {
            panic!("expected period view");
        };
        assert_eq!(values[0].value, None);
    }

    #[test]
    fn histogram_view_densifies_fixed_bins_and_lists_every_entity() {
        let req = request(
            vec![
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
            ],
            "2026-01-01",
            "2026-01-31",
        );
        // person A observed bins 0 and 9 over range [0, 100].
        let rows = vec![
            histogram_row("00000000-0000-0000-0000-00000000000a", 0, 0.0, 100.0, 3),
            histogram_row("00000000-0000-0000-0000-00000000000a", 9, 0.0, 100.0, 1),
        ];
        let MetricResultViewDto::Histogram { values, .. } = build_histogram_view(&req, rows) else {
            panic!("expected histogram view");
        };
        assert_eq!(values.len(), 2);

        let a = &values[0];
        assert_eq!(
            a.entity_id.as_deref(),
            Some("00000000-0000-0000-0000-00000000000a")
        );
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
        assert_eq!(
            b.entity_id.as_deref(),
            Some("00000000-0000-0000-0000-00000000000b")
        );
        assert!(b.bins.is_empty());
    }

    fn pooled_histogram_row(
        repository: &str,
        bin_idx: u32,
        lo: f64,
        hi: f64,
        count: u64,
    ) -> PooledHistogramQueryRow {
        PooledHistogramQueryRow {
            bin_idx,
            group_lo: lo,
            group_hi: hi,
            bin_count: Some(count),
            extra: HashMap::from([
                ("dim_0_value".to_owned(), json!(repository)),
                ("dim_0_label".to_owned(), json!(repository)),
            ]),
        }
    }

    #[test]
    fn pooled_histogram_bins_per_dimension_tuple_without_entity_grain() {
        let rows = vec![
            pooled_histogram_row("acme/api", 0, 0.0, 100.0, 3),
            pooled_histogram_row("acme/api", 9, 0.0, 100.0, 1),
            pooled_histogram_row("acme/web", 0, 5.0, 5.0, 2),
        ];
        let Ok(view) = build_pooled_histogram_view(rows, &["repository".to_owned()]) else {
            panic!("expected the pooled histogram to build");
        };
        let MetricResultViewDto::Histogram { dimensions, values } = view else {
            panic!("expected histogram view");
        };
        assert_eq!(dimensions, vec!["repository".to_owned()]);
        assert_eq!(values.len(), 2);

        let api = &values[0];
        // No entity grain: a pooled row answers for the tuple, not a person.
        assert!(api.entity_id.is_none());
        assert_eq!(api.dimensions[0].value, "acme/api");
        assert_eq!(api.bins.len(), 10);
        assert_eq!(api.bins[0].count, 3);
        assert_eq!(api.bins[9].count, 1);
        assert!((api.bins[9].hi - 100.0).abs() < f64::EPSILON);

        // A tuple whose events all share one value collapses to a single bin,
        // exactly as the per-entity shape does.
        let web = &values[1];
        assert_eq!(web.bins.len(), 1);
        assert_eq!(web.bins[0].count, 2);
    }

    #[test]
    fn histogram_view_collapses_identical_values_to_single_bin() {
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-31",
        );
        let rows = vec![histogram_row(
            "00000000-0000-0000-0000-00000000000a",
            0,
            7.5,
            7.5,
            4,
        )];
        let MetricResultViewDto::Histogram { values, .. } = build_histogram_view(&req, rows) else {
            panic!("expected histogram view");
        };
        assert_eq!(values[0].bins.len(), 1);
        assert!((values[0].bins[0].lo - 7.5).abs() < f64::EPSILON);
        assert!((values[0].bins[0].hi - 7.5).abs() < f64::EPSILON);
        assert_eq!(values[0].bins[0].count, 4);
    }

    #[test]
    fn timeseries_densifies_all_buckets_per_entity() {
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-03",
        );
        let rows = vec![TimeseriesQueryRow {
            entity_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
            bucket_start: "2026-01-02".to_owned(),
            value: Some(3.0),
            is_total: 0,
            rank: None,
            remainder: 0,
            group_label: None,
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
        assert_eq!(points[0].value, None);
        assert_eq!(points[1].value, Some(3.0));
        assert_eq!(points[2].value, None);
    }

    #[test]
    fn ungrouped_timeseries_emits_series_for_entities_without_rows() {
        let req = request(
            vec![
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
            ],
            "2026-01-01",
            "2026-01-02",
        );
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
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-02",
        );
        let mut extra = HashMap::new();
        extra.insert("dim_0_value".to_owned(), json!("cursor"));
        extra.insert("dim_0_label".to_owned(), json!("Cursor"));
        let rows = vec![TimeseriesQueryRow {
            entity_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
            bucket_start: "2026-01-01".to_owned(),
            value: Some(2.0),
            is_total: 0,
            rank: None,
            remainder: 0,
            group_label: None,
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
    fn bounded_timeseries_carries_totals_ranks_and_remainder_metadata() {
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-02",
        );
        let mut dimensions = HashMap::new();
        dimensions.insert("dim_0_value".to_owned(), json!("cursor"));
        dimensions.insert("dim_0_label".to_owned(), json!("Cursor"));
        let rows = vec![
            TimeseriesQueryRow {
                entity_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
                bucket_start: "2026-01-01".to_owned(),
                value: Some(2.0),
                is_total: 0,
                rank: Some(1),
                remainder: 0,
                group_label: None,
                extra: dimensions.clone(),
            },
            TimeseriesQueryRow {
                entity_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
                bucket_start: String::new(),
                value: Some(3.0),
                is_total: 1,
                rank: Some(1),
                remainder: 0,
                group_label: None,
                extra: dimensions,
            },
            TimeseriesQueryRow {
                entity_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
                bucket_start: "2026-01-01".to_owned(),
                value: Some(4.0),
                is_total: 0,
                rank: None,
                remainder: 1,
                group_label: Some("Other".to_owned()),
                extra: HashMap::new(),
            },
            TimeseriesQueryRow {
                entity_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
                bucket_start: String::new(),
                value: Some(5.0),
                is_total: 1,
                rank: None,
                remainder: 1,
                group_label: Some("Other".to_owned()),
                extra: HashMap::new(),
            },
        ];
        let Ok(MetricResultViewDto::Timeseries { series, .. }) =
            build_timeseries_view(&sum_metric(), &req, Bucket::Day, &["tool".to_owned()], rows)
        else {
            panic!("expected timeseries view");
        };
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].rank, Some(1));
        assert_eq!(series[0].total, Some(3.0));
        assert_eq!(series[0].remainder, None);
        assert_eq!(series[1].dimensions.len(), 0);
        assert_eq!(series[1].total, Some(5.0));
        assert_eq!(series[1].remainder, Some(true));
        assert_eq!(series[1].label.as_deref(), Some("Other"));
    }

    #[test]
    fn ranking_rows_keep_query_order_and_unknown_dimensions() {
        let rows = vec![
            RankingQueryRow {
                extra: HashMap::from([
                    ("dim_0_value".to_owned(), json!("cursor")),
                    ("dim_0_label".to_owned(), json!("Cursor")),
                    ("value".to_owned(), json!(10)),
                ]),
            },
            RankingQueryRow {
                extra: HashMap::from([
                    ("dim_0_value".to_owned(), serde_json::Value::Null),
                    ("dim_0_label".to_owned(), serde_json::Value::Null),
                    ("value".to_owned(), json!(0)),
                ]),
            },
        ];
        let groups = build_ranked_groups(&["tool".to_owned()], rows)
            .unwrap_or_else(|error| panic!("expected ranking groups: {error}"));
        assert_eq!(groups[0].rank, 1);
        assert_eq!(groups[0].dimensions[0].value, "cursor");
        assert_eq!(groups[1].rank, 2);
        assert_eq!(groups[1].dimensions[0].value, UNKNOWN_DIMENSION_VALUE);
        assert_eq!(
            groups[1].dimensions[0].label.as_deref(),
            Some(UNKNOWN_DIMENSION_LABEL)
        );
    }

    #[test]
    fn missing_dimension_alias_is_internal_error() {
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-02",
        );
        let rows = vec![TimeseriesQueryRow {
            entity_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
            bucket_start: "2026-01-01".to_owned(),
            value: Some(2.0),
            is_total: 0,
            rank: None,
            remainder: 0,
            group_label: None,
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
            entity_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
            value: Some(1.0),
            source_provider: None,
            source_id: None,
            extra,
        }];
        let dimensions = vec!["tool".to_owned()];
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-31",
        );
        let Ok(MetricResultViewDto::Breakdown { values, .. }) =
            build_breakdown_view(&req, &dimensions, rows, &ExternalSourceRegistry::default())
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
    fn breakdown_repository_uses_hidden_source_context_for_href() -> anyhow::Result<()> {
        let mut extra = HashMap::new();
        extra.insert(
            "dim_0_value".to_owned(),
            serde_json::json!("source-a:group/repository"),
        );
        extra.insert(
            "dim_0_label".to_owned(),
            serde_json::json!("group/repository"),
        );
        let rows = vec![BreakdownQueryRow {
            entity_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
            value: Some(1.0),
            source_provider: Some("github".to_owned()),
            source_id: Some("source-a".to_owned()),
            extra,
        }];
        let dimensions = vec!["repository".to_owned()];
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-31",
        );
        let registry = ExternalSourceRegistry::new(&[crate::config::ExternalSourceConfig {
            id: "source-a".to_owned(),
            provider: crate::config::ExternalSourceProvider::Github,
            web_base_url: "https://code.example.test".to_owned(),
        }])?;

        let MetricResultViewDto::Breakdown { values, .. } =
            build_breakdown_view(&req, &dimensions, rows, &registry)?
        else {
            panic!("expected breakdown view");
        };

        assert_eq!(
            values[0].dimensions[0].href.as_deref(),
            Some("https://code.example.test/group/repository")
        );
        Ok(())
    }

    #[test]
    fn rollup_keeps_contributor_count_and_marks_remainder() {
        let rows = vec![
            RollupQueryRow {
                value: Some(4.0),
                contributing_entity_count: Some(2),
                rank: Some(1),
                remainder: 0,
                group_label: None,
                extra: HashMap::from([
                    ("dim_0_value".to_owned(), json!("example/repository")),
                    ("dim_0_label".to_owned(), json!("Example repository")),
                ]),
            },
            RollupQueryRow {
                value: Some(3.0),
                contributing_entity_count: Some(2),
                rank: None,
                remainder: 1,
                group_label: Some("Other".to_owned()),
                extra: HashMap::new(),
            },
        ];

        let Ok(MetricResultViewDto::Rollup { values, .. }) =
            build_rollup_view(&["repository".to_owned()], rows)
        else {
            panic!("expected rollup view");
        };
        assert_eq!(values[0].contributing_entity_count, 2);
        assert_eq!(values[0].rank, Some(1));
        assert_eq!(values[1].dimensions.len(), 0);
        assert_eq!(values[1].remainder, Some(true));
        assert_eq!(values[1].label.as_deref(), Some("Other"));
    }

    fn selection(metric_key: &str) -> super::super::dto::MetricResultSelectionDto {
        super::super::dto::MetricResultSelectionDto {
            metric_key: metric_key.to_owned(),
            entity: super::super::dto::MetricResultsEntityDto::Person {
                ids: vec!["person@example.com".to_owned()],
            },
            period: super::super::dto::MetricResultsPeriodDto {
                from: "2026-07-01".to_owned(),
                to: "2026-07-28".to_owned(),
            },
            filters: Vec::new(),
        }
    }

    #[test]
    fn metric_result_wire_shape_is_flat_with_computation_tag() {
        let sum = build_metric_result(&sum_metric(), Vec::new(), selection("ai.accepted_lines"));
        let sum_json = serde_json::to_value(&sum).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(sum_json["computation"], "sum");
        assert_eq!(sum_json["metric_key"], "ai.accepted_lines");
        assert_eq!(sum_json["format"], "integer");
        assert!(sum_json.get("scale").is_none());
        assert_eq!(sum_json["selection"]["metric_key"], "ai.accepted_lines");
        assert_eq!(sum_json["selection"]["entity"]["type"], "person");
        assert_eq!(
            sum_json["selection"]["entity"]["ids"][0],
            "person@example.com"
        );
        assert_eq!(sum_json["selection"]["period"]["from"], "2026-07-01");
        assert_eq!(sum_json["selection"]["period"]["to"], "2026-07-28");
        assert_eq!(sum_json["selection"]["filters"], serde_json::json!([]));
        assert!(
            sum_json.get("drilldown").is_none(),
            "absent drilldown capability must be omitted from the wire shape"
        );

        let ratio = build_metric_result(&ratio_metric(), Vec::new(), selection("ai.ratio"));
        let ratio_json = serde_json::to_value(&ratio).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ratio_json["computation"], "ratio");
        assert_eq!(ratio_json["scale"], 100.0);

        let median = build_metric_result(&median_metric(), Vec::new(), selection("ai.median"));
        let median_json = serde_json::to_value(&median).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(median_json["computation"], "median");
        assert!(median_json.get("scale").is_none());

        let distinct = build_metric_result(
            &distinct_count_metric(),
            Vec::new(),
            selection("ai.distinct"),
        );
        let distinct_json = serde_json::to_value(&distinct).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(distinct_json["computation"], "distinct_count");
        assert!(distinct_json.get("scale").is_none());

        let percentile =
            build_metric_result(&percentile_metric(), Vec::new(), selection("ai.percentile"));
        let percentile_json = serde_json::to_value(&percentile).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(percentile_json["computation"], "percentile");
        assert_eq!(percentile_json["q"], 0.75);
        assert!(percentile_json.get("scale").is_none());
    }

    #[test]
    fn histogram_wire_shape_uses_view_tag() {
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-31",
        );
        let view = build_histogram_view(
            &req,
            vec![histogram_row(
                "00000000-0000-0000-0000-00000000000a",
                0,
                1.0,
                1.0,
                2,
            )],
        );
        let json = serde_json::to_value(&view).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(json["view"], "histogram");
        assert_eq!(
            json["values"][0]["entity_id"],
            "00000000-0000-0000-0000-00000000000a"
        );
        assert_eq!(json["values"][0]["bins"][0]["count"], 2);
        assert_eq!(json["values"][0]["bins"][0]["lo"], 1.0);
        assert_eq!(json["values"][0]["bins"][0]["hi"], 1.0);
    }

    #[test]
    fn view_size_counts_histogram_bins() {
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-31",
        );
        let view = build_histogram_view(
            &req,
            vec![histogram_row(
                "00000000-0000-0000-0000-00000000000a",
                2,
                0.0,
                10.0,
                1,
            )],
        );
        assert_eq!(view_size(&view), 10);
    }

    #[test]
    fn view_size_counts_densified_points() {
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000a"],
            "2026-01-01",
            "2026-01-10",
        );
        let Ok(view) = build_timeseries_view(&sum_metric(), &req, Bucket::Day, &[], Vec::new())
        else {
            panic!("expected timeseries view");
        };
        assert_eq!(view_size(&view), 11);
    }

    #[test]
    fn view_limit_rejects_cardinality_dependent_results() {
        let values = (0..=row_limit())
            .map(|index| PeriodValueDto {
                entity_id: Uuid::from_u128(index as u128 + 1).to_string(),
                value: Some(1.0),
            })
            .collect();
        let view = MetricResultViewDto::Period { values };
        assert!(enforce_view_row_limit(&view, "metrics[0].views[0]").is_err());
    }

    #[test]
    fn an_error_view_never_trips_the_row_limit() {
        let view = MetricResultViewDto::Error {
            code: crate::domain::metric_results::dto::MetricViewErrorCode::QueryFailed,
            message: "generic".to_owned(),
        };
        assert!(enforce_view_row_limit(&view, "metrics[0].views[0]").is_ok());
    }

    #[test]
    fn missing_value_ignores_the_transform() {
        let mut def = sum_metric();
        def.transform = Some(ValueTransform {
            multiplier: Some(-1.0),
            offset: Some(100.0),
            clamp_min: Some(0.0),
            clamp_max: Some(100.0),
        });
        let req = request(
            vec!["00000000-0000-0000-0000-00000000000c"],
            "2026-01-01",
            "2026-01-31",
        );
        let MetricResultViewDto::Period { values } = build_period_view(&def, &req, vec![]) else {
            panic!("expected period view");
        };
        assert_eq!(values[0].value, None);
    }
}
