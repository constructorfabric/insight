use std::collections::BTreeSet;

use chrono::{Datelike, Duration, NaiveDate};
use sea_orm::DatabaseConnection;
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::error::MetricError;
use crate::domain::metric_definitions::{ComputationSpec, MetricDefinition, load_definitions};

use super::dto::{MetricDimensionFilterRequest, MetricResultsRequest, MetricViewRequest};
use super::view::Bucket;

const ROW_LIMIT: usize = 5000;
const MAX_METRICS: usize = 50;
const MAX_ENTITY_IDS: usize = 1000;
const MAX_PERIOD_DAYS: i64 = 400;
const MAX_FILTERS: usize = 10;
const MAX_FILTER_VALUES: usize = 100;
const MAX_FILTER_VALUE_BYTES: usize = 512;

/// Server-owned histogram resolution: fixed-width bins over each entity's
/// own exact [min, max]. A fixed count keeps responses deterministic and
/// the projected-row math trivial; the choice of range strategy lives in
/// the compiler's bounds CTE and can evolve without touching the wire.
pub(crate) const HISTOGRAM_BINS: usize = 10;

#[derive(Debug)]
pub struct ValidatedMetricResultsRequest {
    pub entity_type: String,
    pub entity_ids: Vec<String>,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub metrics: Vec<ValidatedMetricRequest>,
}

#[derive(Debug)]
pub struct ValidatedMetricRequest {
    pub def: MetricDefinition,
    pub filters: Vec<ValidatedDimensionFilter>,
    pub views: Vec<ValidatedMetricView>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ValidatedDimensionFilter {
    pub dimension: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ValidatedMetricView {
    Period,
    Peer {
        cohort_key: String,
    },
    Timeseries {
        bucket: Bucket,
        dimensions: Vec<String>,
    },
    Breakdown {
        dimensions: Vec<String>,
    },
    Histogram,
}

struct RequestShape {
    entity_type: String,
    entity_ids: Vec<String>,
    from: NaiveDate,
    to: NaiveDate,
    metric_keys: Vec<String>,
}

pub async fn validate_request(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    req: MetricResultsRequest,
) -> Result<ValidatedMetricResultsRequest, CanonicalError> {
    let shape = validate_request_shape(&req)?;
    let RequestShape {
        entity_type,
        entity_ids,
        from,
        to,
        metric_keys,
    } = shape;

    let mut definitions = load_definitions(db, tenant_id, &metric_keys).await?;
    let mut metrics = Vec::with_capacity(req.metrics.len());

    for metric in req.metrics {
        let metric_key = metric.metric_key.trim();
        let def = definitions.remove(metric_key).ok_or_else(|| {
            tracing::error!(metric_key = %metric_key, "definition missing after successful load");
            CanonicalError::internal("metric definition lookup failed").create()
        })?;
        if def.base.entity_type != entity_type {
            return invalid(
                "entity.type",
                format!(
                    "metric {} is defined for entity type {}",
                    def.key(),
                    def.base.entity_type
                ),
            );
        }
        if metric.views.is_empty() {
            return invalid(
                "metrics.views",
                format!("metric {} must request at least one view", def.key()),
            );
        }

        let filters = validate_filters(&def, metric.filters)?;
        let mut view_kinds = BTreeSet::new();
        let mut views = Vec::with_capacity(metric.views.len());
        for view in metric.views {
            let kind = view.kind();
            if !view_kinds.insert(kind) {
                return invalid(
                    "metrics.views",
                    format!("metric {} has duplicate {kind:?} view", def.key()),
                );
            }
            views.push(validate_view(&def, view)?);
        }

        metrics.push(ValidatedMetricRequest {
            def,
            filters,
            views,
        });
    }

    let validated = ValidatedMetricResultsRequest {
        entity_type,
        entity_ids,
        from,
        to,
        metrics,
    };
    validate_projected_row_limit(&validated)?;
    Ok(validated)
}

fn validate_request_shape(req: &MetricResultsRequest) -> Result<RequestShape, CanonicalError> {
    if req.metrics.is_empty() {
        return invalid("metrics", "metrics must not be empty");
    }
    if req.metrics.len() > MAX_METRICS {
        return invalid(
            "metrics",
            format!("at most {MAX_METRICS} metrics per request"),
        );
    }

    let entity_type = normalize_entity_type(&req.entity.r#type)?;
    let entity_ids = normalize_entity_ids(&entity_type, &req.entity.ids)?;
    if entity_ids.len() > MAX_ENTITY_IDS {
        return invalid(
            "entity.ids",
            format!("at most {MAX_ENTITY_IDS} entity ids per request"),
        );
    }
    let from = parse_date("period.from", &req.period.from)?;
    let to = parse_date("period.to", &req.period.to)?;
    if from > to {
        return invalid("period", "period.from must be before or equal to period.to");
    }
    if (to - from).num_days() >= MAX_PERIOD_DAYS {
        return invalid(
            "period",
            format!("period must not exceed {MAX_PERIOD_DAYS} days"),
        );
    }

    let mut seen_metric_keys = BTreeSet::new();
    let mut metric_keys = Vec::with_capacity(req.metrics.len());
    for metric in &req.metrics {
        let metric_key = metric.metric_key.trim();
        if metric_key.is_empty() {
            return invalid("metrics.metric_key", "metric_key must not be empty");
        }
        if !seen_metric_keys.insert(metric_key.to_owned()) {
            return invalid(
                "metrics.metric_key",
                format!("duplicate metric key: {metric_key}"),
            );
        }
        metric_keys.push(metric_key.to_owned());
    }

    Ok(RequestShape {
        entity_type,
        entity_ids,
        from,
        to,
        metric_keys,
    })
}

pub const fn row_limit() -> usize {
    ROW_LIMIT
}

// One past the response cap: a query returning ROW_LIMIT + 1 rows proves
// truncation, which the final enforce_row_limit pass then converts into a
// whole-request failure instead of a silently clipped result.
pub const fn query_row_limit() -> usize {
    ROW_LIMIT + 1
}

pub fn metric_result_too_large() -> CanonicalError {
    MetricError::invalid_argument()
        .with_field_violation(
            "metric_results",
            "Requested metric result exceeds the row limit. Reduce the date range, entities, metrics, or dimensions.",
            "metric_result_too_large",
        )
        .create()
}

fn validate_view(
    def: &MetricDefinition,
    view: MetricViewRequest,
) -> Result<ValidatedMetricView, CanonicalError> {
    match view {
        MetricViewRequest::Period => Ok(ValidatedMetricView::Period),
        MetricViewRequest::Peer { cohort_key } => {
            let cohort_key = match cohort_key {
                Some(key) => {
                    let key = normalize_key("metrics.views.cohort_key", &key)?;
                    if def.base.peer_cohort_key.as_deref() != Some(key.as_str()) {
                        return Err(MetricError::invalid_argument()
                            .with_field_violation(
                                "metrics.views.cohort_key",
                                format!("cohort {key} is not declared for metric {}", def.key()),
                                "INVALID",
                            )
                            .create());
                    }
                    key
                }
                None => def.base.peer_cohort_key.clone().ok_or_else(|| {
                    MetricError::invalid_argument()
                        .with_field_violation(
                            "metrics.views.cohort_key",
                            format!("metric {} has no default peer cohort", def.key()),
                            "INVALID",
                        )
                        .create()
                })?,
            };
            Ok(ValidatedMetricView::Peer { cohort_key })
        }
        MetricViewRequest::Timeseries { bucket, dimensions } => {
            Ok(ValidatedMetricView::Timeseries {
                bucket: bucket.unwrap_or(Bucket::Day),
                dimensions: validate_dimensions(def, "metrics.views.dimensions", dimensions)?,
            })
        }
        MetricViewRequest::Breakdown { dimensions } => {
            if dimensions.is_empty() {
                return invalid(
                    "metrics.views.dimensions",
                    format!(
                        "metric {} breakdown dimensions must not be empty",
                        def.key()
                    ),
                );
            }
            Ok(ValidatedMetricView::Breakdown {
                dimensions: validate_dimensions(def, "metrics.views.dimensions", dimensions)?,
            })
        }
        MetricViewRequest::Histogram => {
            // Histograms bin per-event observation values; only median
            // metrics have event-grain observations to bin.
            if !matches!(def.spec, ComputationSpec::Median { .. }) {
                return invalid(
                    "metrics.views",
                    format!(
                        "metric {} does not support the histogram view; it requires a median computation",
                        def.key()
                    ),
                );
            }
            Ok(ValidatedMetricView::Histogram)
        }
    }
}

fn validate_dimensions(
    def: &MetricDefinition,
    field: &'static str,
    dimensions: Vec<String>,
) -> Result<Vec<String>, CanonicalError> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(dimensions.len());
    for dimension in dimensions {
        let dimension = normalize_key(field, &dimension)?;
        if !seen.insert(dimension.clone()) {
            return invalid(field, format!("duplicate dimension: {dimension}"));
        }
        let Some(valid_dimension) = def.allowed_dimension(&dimension) else {
            return invalid(
                field,
                format!(
                    "metric {} does not support dimension {dimension}",
                    def.key()
                ),
            );
        };
        out.push(valid_dimension.to_owned());
    }
    Ok(out)
}

fn validate_filters(
    def: &MetricDefinition,
    filters: Vec<MetricDimensionFilterRequest>,
) -> Result<Vec<ValidatedDimensionFilter>, CanonicalError> {
    if filters.len() > MAX_FILTERS {
        return invalid(
            "metrics.filters",
            format!("at most {MAX_FILTERS} dimension filters per metric"),
        );
    }
    let mut seen_dimensions = BTreeSet::new();
    let mut out = Vec::with_capacity(filters.len());
    for filter in filters {
        let Some(dimension) =
            validate_dimensions(def, "metrics.filters.dimension", vec![filter.dimension])?
                .into_iter()
                .next()
        else {
            return invalid("metrics.filters.dimension", "dimension must not be empty");
        };
        if !seen_dimensions.insert(dimension.clone()) {
            return invalid(
                "metrics.filters.dimension",
                format!("duplicate dimension filter: {dimension}"),
            );
        }
        if filter.values.is_empty() {
            return invalid(
                "metrics.filters.values",
                format!("filter {dimension} must contain at least one value"),
            );
        }
        if filter.values.len() > MAX_FILTER_VALUES {
            return invalid(
                "metrics.filters.values",
                format!("filter {dimension} supports at most {MAX_FILTER_VALUES} values"),
            );
        }
        let mut seen_values = BTreeSet::new();
        for value in filter.values {
            let value = value.trim();
            if value.is_empty() {
                return invalid("metrics.filters.values", "filter value must not be empty");
            }
            if value.len() > MAX_FILTER_VALUE_BYTES {
                return invalid(
                    "metrics.filters.values",
                    format!("filter value must not exceed {MAX_FILTER_VALUE_BYTES} bytes"),
                );
            }
            seen_values.insert(value.to_owned());
        }
        out.push(ValidatedDimensionFilter {
            dimension,
            values: seen_values.into_iter().collect(),
        });
    }
    out.sort();
    Ok(out)
}

fn normalize_entity_type(entity_type: &str) -> Result<String, CanonicalError> {
    normalize_key("entity.type", entity_type)
}

fn normalize_entity_ids(
    entity_type: &str,
    entity_ids: &[String],
) -> Result<Vec<String>, CanonicalError> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(entity_ids.len());
    for entity_id in entity_ids {
        let entity_id = normalize_entity_id(entity_type, entity_id);
        if entity_id.is_empty() {
            continue;
        }
        if seen.insert(entity_id.clone()) {
            out.push(entity_id);
        }
    }
    if out.is_empty() {
        return invalid("entity.ids", "entity.ids must not be empty");
    }
    Ok(out)
}

// Id normalization is a property of the entity type: person ids are emails
// and the observation sources emit them lowercased, so equality requires
// lowercasing here too. Other entity types keep their casing.
fn normalize_entity_id(entity_type: &str, entity_id: &str) -> String {
    let trimmed = entity_id.trim();
    match entity_type {
        "person" => trimmed.to_ascii_lowercase(),
        _ => trimmed.to_owned(),
    }
}

fn normalize_key(field: &'static str, value: &str) -> Result<String, CanonicalError> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return invalid(field, "value must not be empty");
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        || value
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_digit() || *b == b'_')
    {
        return invalid(field, "expected lowercase snake case");
    }
    Ok(value)
}

fn validate_projected_row_limit(req: &ValidatedMetricResultsRequest) -> Result<(), CanonicalError> {
    let mut projected = 0usize;

    for metric in &req.metrics {
        for view in &metric.views {
            match view {
                ValidatedMetricView::Period | ValidatedMetricView::Peer { .. } => {
                    projected = projected.saturating_add(req.entity_ids.len());
                }
                ValidatedMetricView::Timeseries { bucket, .. } => {
                    projected = projected.saturating_add(
                        req.entity_ids
                            .len()
                            .saturating_mul(enumerate_buckets(req.from, req.to, *bucket).len()),
                    );
                }
                ValidatedMetricView::Histogram => {
                    projected = projected
                        .saturating_add(req.entity_ids.len().saturating_mul(HISTOGRAM_BINS));
                }
                ValidatedMetricView::Breakdown { .. } => {}
            }
        }
    }

    if projected > ROW_LIMIT {
        return Err(metric_result_too_large());
    }
    Ok(())
}

pub fn enumerate_buckets(from: NaiveDate, to: NaiveDate, bucket: Bucket) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut day = from;
    while day <= to {
        let bucket_start = match bucket {
            Bucket::Day => day,
            Bucket::Week => day - Duration::days(i64::from(day.weekday().num_days_from_monday())),
            Bucket::Month => NaiveDate::from_ymd_opt(day.year(), day.month(), 1).unwrap_or(day),
        };
        if seen.insert(bucket_start) {
            out.push(bucket_start.to_string());
        }
        day += Duration::days(1);
    }
    out
}

fn parse_date(field: &'static str, value: &str) -> Result<NaiveDate, CanonicalError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        MetricError::invalid_argument()
            .with_field_violation(field, "expected YYYY-MM-DD", "INVALID")
            .create()
    })
}

fn invalid<T>(field: &'static str, message: impl Into<String>) -> Result<T, CanonicalError> {
    Err(MetricError::invalid_argument()
        .with_field_violation(field, message.into(), "INVALID")
        .create())
}

#[cfg(test)]
mod tests {
    use super::super::dto::MetricResultsEntity;
    use super::*;
    use crate::domain::metric_definitions::definition::{
        ComputationSpec, MetricBase, MetricDefinition, MetricDirection, MetricFormat, MetricInput,
        MetricInputRole, ObservationRelation,
    };

    fn shape_request(
        entity_ids: Vec<&str>,
        from: &str,
        to: &str,
        metric_keys: Vec<&str>,
    ) -> MetricResultsRequest {
        MetricResultsRequest {
            entity: MetricResultsEntity {
                r#type: "person".to_owned(),
                ids: entity_ids.into_iter().map(str::to_owned).collect(),
            },
            period: super::super::dto::MetricResultsPeriod {
                from: from.to_owned(),
                to: to.to_owned(),
            },
            metrics: metric_keys
                .into_iter()
                .map(|key| super::super::dto::MetricRequest {
                    metric_key: key.to_owned(),
                    filters: vec![],
                    views: vec![MetricViewRequest::Period],
                })
                .collect(),
        }
    }

    fn sum_definition(dimensions: Vec<&str>) -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: MetricBase {
                key: "ai.accepted_lines".to_owned(),
                label: "AI-added lines".to_owned(),
                description: None,
                explanation: None,
                entity_type: "person".to_owned(),
                format: MetricFormat::Integer,
                unit: None,
                direction: MetricDirection::HigherIsBetter,
                peer_cohort_key: Some("org_unit".to_owned()),
                allowed_dimensions: dimensions.into_iter().map(str::to_owned).collect(),
            },
            spec: ComputationSpec::Sum {
                value: MetricInput {
                    role: MetricInputRole::Value,
                    observation_relation: ObservationRelation::parse("ai_metric_observations")
                        .unwrap_or_else(|| panic!("fixture relation must parse")),
                    source_key: "ai_usage".to_owned(),
                    measure_key: "accepted_lines".to_owned(),
                },
            },
        }
    }

    fn fixture_input(measure_key: &str, role: MetricInputRole) -> MetricInput {
        MetricInput {
            role,
            observation_relation: ObservationRelation::parse("ai_metric_observations")
                .unwrap_or_else(|| panic!("fixture relation must parse")),
            source_key: "ai_usage".to_owned(),
            measure_key: measure_key.to_owned(),
        }
    }

    fn ratio_definition() -> MetricDefinition {
        let mut def = sum_definition(vec![]);
        def.spec = ComputationSpec::Ratio {
            numerator: fixture_input("accepted_edit_actions", MetricInputRole::Numerator),
            denominator: fixture_input("tool_use_offered", MetricInputRole::Denominator),
            scale: 100.0,
        };
        def
    }

    fn median_definition() -> MetricDefinition {
        let mut def = sum_definition(vec![]);
        def.spec = ComputationSpec::Median {
            value: fixture_input("pr_cycle_hours", MetricInputRole::Value),
        };
        def
    }

    fn day(value: &str) -> NaiveDate {
        match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
            Ok(date) => date,
            Err(error) => panic!("bad test date {value}: {error}"),
        }
    }

    #[test]
    fn shape_accepts_valid_request() {
        let Ok(shape) = validate_request_shape(&shape_request(
            vec!["A@x.io "],
            "2026-01-01",
            "2026-01-31",
            vec!["ai.x"],
        )) else {
            panic!("expected valid shape");
        };
        assert_eq!(shape.entity_type, "person");
        assert_eq!(shape.entity_ids, vec!["a@x.io".to_owned()]);
        assert_eq!(shape.metric_keys, vec!["ai.x".to_owned()]);
    }

    #[test]
    fn shape_rejects_too_many_metrics() {
        let keys: Vec<String> = (0..=MAX_METRICS).map(|i| format!("ai.m{i}")).collect();
        let req = shape_request(
            vec!["a@x.io"],
            "2026-01-01",
            "2026-01-31",
            keys.iter().map(String::as_str).collect(),
        );
        assert!(validate_request_shape(&req).is_err());
    }

    #[test]
    fn shape_rejects_too_many_entity_ids() {
        let ids: Vec<String> = (0..=MAX_ENTITY_IDS).map(|i| format!("p{i}@x.io")).collect();
        let req = shape_request(
            ids.iter().map(String::as_str).collect(),
            "2026-01-01",
            "2026-01-31",
            vec!["ai.x"],
        );
        assert!(validate_request_shape(&req).is_err());
    }

    #[test]
    fn shape_rejects_oversized_period_before_enumeration() {
        let req = shape_request(vec!["a@x.io"], "0001-01-01", "9999-12-31", vec!["ai.x"]);
        assert!(validate_request_shape(&req).is_err());
    }

    #[test]
    fn shape_rejects_reversed_period() {
        let req = shape_request(vec!["a@x.io"], "2026-02-01", "2026-01-01", vec!["ai.x"]);
        assert!(validate_request_shape(&req).is_err());
    }

    #[test]
    fn shape_rejects_duplicate_metric_keys() {
        let req = shape_request(
            vec!["a@x.io"],
            "2026-01-01",
            "2026-01-31",
            vec!["ai.x", "ai.x"],
        );
        assert!(validate_request_shape(&req).is_err());
    }

    #[test]
    fn shape_rejects_all_blank_entity_ids() {
        let req = shape_request(vec![" ", ""], "2026-01-01", "2026-01-31", vec!["ai.x"]);
        assert!(validate_request_shape(&req).is_err());
    }

    #[test]
    fn person_entity_ids_are_lowercased() {
        let Ok(ids) = normalize_entity_ids("person", &[" A@X.io ".to_owned()]) else {
            panic!("expected normalized ids");
        };
        assert_eq!(ids, vec!["a@x.io".to_owned()]);
    }

    #[test]
    fn non_person_entity_ids_keep_case() {
        let Ok(ids) = normalize_entity_ids("repo", &[" Org/Repo-Name ".to_owned()]) else {
            panic!("expected normalized ids");
        };
        assert_eq!(ids, vec!["Org/Repo-Name".to_owned()]);
    }

    #[test]
    fn normalize_key_enforces_snake_case() {
        assert_eq!(normalize_key("f", " Tool ").ok().as_deref(), Some("tool"));
        assert!(normalize_key("f", "").is_err());
        assert!(normalize_key("f", "1tool").is_err());
        assert!(normalize_key("f", "_tool").is_err());
        assert!(normalize_key("f", "tool-x").is_err());
        assert!(normalize_key("f", "tool x").is_err());
        assert_eq!(
            normalize_key("f", "org_unit2").ok().as_deref(),
            Some("org_unit2")
        );
    }

    #[test]
    fn validate_view_rejects_undeclared_dimension() {
        let def = sum_definition(vec!["tool"]);
        let view = MetricViewRequest::Breakdown {
            dimensions: vec!["surface".to_owned()],
        };
        assert!(validate_view(&def, view).is_err());
    }

    #[test]
    fn validate_view_defaults_timeseries_bucket_to_day() {
        let def = sum_definition(vec!["tool"]);
        let view = MetricViewRequest::Timeseries {
            bucket: None,
            dimensions: vec![],
        };
        match validate_view(&def, view) {
            Ok(ValidatedMetricView::Timeseries { bucket, .. }) => assert_eq!(bucket, Bucket::Day),
            other => panic!("expected timeseries, got {other:?}"),
        }
    }

    #[test]
    fn validate_view_peer_uses_definition_default_cohort() {
        let def = sum_definition(vec![]);
        match validate_view(&def, MetricViewRequest::Peer { cohort_key: None }) {
            Ok(ValidatedMetricView::Peer { cohort_key }) => assert_eq!(cohort_key, "org_unit"),
            other => panic!("expected peer, got {other:?}"),
        }
    }

    #[test]
    fn validate_view_peer_accepts_explicit_declared_cohort() {
        let def = sum_definition(vec![]);
        let view = MetricViewRequest::Peer {
            cohort_key: Some("org_unit".to_owned()),
        };
        match validate_view(&def, view) {
            Ok(ValidatedMetricView::Peer { cohort_key }) => assert_eq!(cohort_key, "org_unit"),
            other => panic!("expected peer, got {other:?}"),
        }
    }

    #[test]
    fn validate_view_peer_rejects_undeclared_cohort() {
        let def = sum_definition(vec![]);
        let view = MetricViewRequest::Peer {
            cohort_key: Some("team".to_owned()),
        };
        assert!(validate_view(&def, view).is_err());
    }

    #[test]
    fn validate_view_rejects_empty_breakdown_dimensions() {
        let def = sum_definition(vec!["tool"]);
        let view = MetricViewRequest::Breakdown { dimensions: vec![] };
        assert!(validate_view(&def, view).is_err());
    }

    #[test]
    fn filters_are_trimmed_deduplicated_and_sorted() {
        let def = sum_definition(vec!["source", "repository"]);
        let filters = vec![
            MetricDimensionFilterRequest {
                dimension: " source ".to_owned(),
                values: vec![
                    "github".to_owned(),
                    " bitbucket ".to_owned(),
                    "github".to_owned(),
                ],
            },
            MetricDimensionFilterRequest {
                dimension: "repository".to_owned(),
                values: vec!["z/repo".to_owned(), "a/repo".to_owned()],
            },
        ];
        let Ok(filters) = validate_filters(&def, filters) else {
            panic!("expected valid filters");
        };
        assert_eq!(
            filters,
            vec![
                ValidatedDimensionFilter {
                    dimension: "repository".to_owned(),
                    values: vec!["a/repo".to_owned(), "z/repo".to_owned()],
                },
                ValidatedDimensionFilter {
                    dimension: "source".to_owned(),
                    values: vec!["bitbucket".to_owned(), "github".to_owned()],
                },
            ]
        );
    }

    #[test]
    fn filters_reject_invalid_shapes() {
        let def = sum_definition(vec!["source"]);
        let filter = |dimension: &str, values: Vec<String>| MetricDimensionFilterRequest {
            dimension: dimension.to_owned(),
            values,
        };
        assert!(
            validate_filters(
                &def,
                vec![
                    filter("source", vec!["github".to_owned()]),
                    filter("source", vec!["gitlab".to_owned()]),
                ],
            )
            .is_err()
        );
        assert!(validate_filters(&def, vec![filter("source", vec![])]).is_err());
        assert!(
            validate_filters(
                &def,
                vec![filter(
                    "source",
                    (0..=MAX_FILTER_VALUES)
                        .map(|index| index.to_string())
                        .collect(),
                )],
            )
            .is_err()
        );
        assert!(validate_filters(&def, vec![filter("source", vec![" ".to_owned()])]).is_err());
        assert!(
            validate_filters(
                &def,
                vec![filter(
                    "source",
                    vec!["x".repeat(MAX_FILTER_VALUE_BYTES + 1)],
                )],
            )
            .is_err()
        );
        assert!(
            validate_filters(
                &def,
                (0..=MAX_FILTERS)
                    .map(|index| filter(&format!("d{index}"), vec!["x".to_owned()]))
                    .collect(),
            )
            .is_err()
        );
    }

    #[test]
    fn validate_view_gates_histogram_on_median_computation() {
        // Histograms bin per-event values; sum/ratio observations are
        // day-aggregated, so binning them would present aggregates as events.
        assert!(validate_view(&sum_definition(vec![]), MetricViewRequest::Histogram).is_err());
        assert!(validate_view(&ratio_definition(), MetricViewRequest::Histogram).is_err());
        assert!(matches!(
            validate_view(&median_definition(), MetricViewRequest::Histogram),
            Ok(ValidatedMetricView::Histogram)
        ));
    }

    #[test]
    fn enumerate_day_buckets_counts_days() {
        let buckets = enumerate_buckets(day("2026-01-30"), day("2026-02-02"), Bucket::Day);
        assert_eq!(
            buckets,
            vec!["2026-01-30", "2026-01-31", "2026-02-01", "2026-02-02"]
        );
    }

    #[test]
    fn enumerate_week_buckets_start_monday() {
        let buckets = enumerate_buckets(day("2026-07-01"), day("2026-07-14"), Bucket::Week);
        assert_eq!(buckets, vec!["2026-06-29", "2026-07-06", "2026-07-13"]);
    }

    #[test]
    fn enumerate_month_buckets_cross_year() {
        let buckets = enumerate_buckets(day("2025-12-15"), day("2026-02-01"), Bucket::Month);
        assert_eq!(buckets, vec!["2025-12-01", "2026-01-01", "2026-02-01"]);
    }

    #[test]
    fn enumerate_single_day_range() {
        let buckets = enumerate_buckets(day("2026-07-02"), day("2026-07-02"), Bucket::Week);
        assert_eq!(buckets, vec!["2026-06-29"]);
    }

    #[test]
    fn projected_row_limit_counts_timeseries_buckets() {
        let def = sum_definition(vec![]);
        let validated = ValidatedMetricResultsRequest {
            entity_type: "person".to_owned(),
            entity_ids: (0..100).map(|i| format!("p{i}@x.io")).collect(),
            from: day("2026-01-01"),
            to: day("2026-03-31"),
            metrics: vec![ValidatedMetricRequest {
                def,
                filters: vec![],
                views: vec![ValidatedMetricView::Timeseries {
                    bucket: Bucket::Day,
                    dimensions: vec![],
                }],
            }],
        };
        assert!(validate_projected_row_limit(&validated).is_err());
    }

    #[test]
    fn projected_row_limit_counts_histogram_bins() {
        // 501 entities × 10 bins > 5000 projected rows.
        let validated = ValidatedMetricResultsRequest {
            entity_type: "person".to_owned(),
            entity_ids: (0..501).map(|i| format!("p{i}@x.io")).collect(),
            from: day("2026-01-01"),
            to: day("2026-01-31"),
            metrics: vec![ValidatedMetricRequest {
                def: median_definition(),
                filters: vec![],
                views: vec![ValidatedMetricView::Histogram],
            }],
        };
        assert!(validate_projected_row_limit(&validated).is_err());
    }

    #[test]
    fn projected_row_limit_allows_small_requests() {
        let def = sum_definition(vec![]);
        let validated = ValidatedMetricResultsRequest {
            entity_type: "person".to_owned(),
            entity_ids: vec!["a@x.io".to_owned()],
            from: day("2026-01-01"),
            to: day("2026-01-31"),
            metrics: vec![ValidatedMetricRequest {
                def,
                filters: vec![],
                views: vec![
                    ValidatedMetricView::Period,
                    ValidatedMetricView::Peer {
                        cohort_key: "org_unit".to_owned(),
                    },
                ],
            }],
        };
        assert!(validate_projected_row_limit(&validated).is_ok());
    }
}
