use std::collections::HashMap;
use std::fmt::Write;

use serde::Deserialize;

use super::validation::{ValidatedMetricResultsRequest, ValidatedMetricView, query_row_limit};
use super::view::Bucket;
use crate::domain::metric_definitions::{
    CohortSource, ComputationSpec, MetricDefinition, ObservationSource,
};

pub(crate) const UNKNOWN_DIMENSION_VALUE: &str = "__unknown__";
pub(crate) const UNKNOWN_DIMENSION_LABEL: &str = "Unknown";

/// Minimum peer-pool size for percentile disclosure. Below this, quartiles
/// over a handful of people are noise presented as signal (someone is always
/// "bottom 25%" of three), and with n=2 the "median" discloses the one
/// colleague's value. Enforced here, server-side, so every consumer inherits
/// it: the peer view still reports `n`, but p25/median/p75/min/max come back
/// NULL and clients render "no peer data".
pub(crate) const MIN_PEER_N: u32 = 5;

#[derive(Debug)]
pub struct CompiledQuery {
    pub sql: String,
    pub params: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PeriodQueryRow {
    pub entity_id: String,
    pub value: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct TimeseriesQueryRow {
    pub entity_id: String,
    pub bucket_start: String,
    pub value: Option<f64>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct PeerQueryRow {
    pub entity_id: String,
    pub target_value: Option<f64>,
    pub p25: Option<f64>,
    pub median: Option<f64>,
    pub p75: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    #[serde(default, deserialize_with = "optional_u64")]
    pub n: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct BreakdownQueryRow {
    pub entity_id: String,
    pub value: Option<f64>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

pub fn compile_view_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    view: &ValidatedMetricView,
) -> CompiledQuery {
    match view {
        ValidatedMetricView::Period => compile_period_query(def, req),
        ValidatedMetricView::Peer { cohort_key } => compile_peer_query(def, req, cohort_key),
        ValidatedMetricView::Timeseries { bucket, dimensions } => {
            compile_timeseries_query(def, req, *bucket, dimensions)
        }
        ValidatedMetricView::Breakdown { dimensions } => {
            compile_breakdown_query(def, req, dimensions)
        }
    }
}

fn compile_period_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
) -> CompiledQuery {
    let mut params = metric_params(def, req);
    params.extend(req.entity_ids.iter().cloned());
    let entities = placeholders(req.entity_ids.len());
    let observation_table = observation_table(def.observation_source());
    let limit = query_row_limit();
    let sql = match &def.spec {
        ComputationSpec::Sum { .. } => format!(
            r"
            SELECT
                entity_id,
                sumIf(value, value IS NOT NULL) AS value
            FROM {observation_table}
            WHERE {metric_where}
              AND entity_id IN ({entities})
            GROUP BY entity_id
            LIMIT {limit}
            ",
            metric_where = metric_where(def),
        ),
        ComputationSpec::Ratio { scale, .. } => format!(
            r"
            SELECT
                entity_id,
                {scale} * sumIf(value, measure_key = ? AND value IS NOT NULL)
                    / nullIf(sumIf(value, measure_key = ? AND value IS NOT NULL), 0) AS value
            FROM {observation_table}
            WHERE {metric_where}
              AND entity_id IN ({entities})
            GROUP BY entity_id
            LIMIT {limit}
            ",
            scale = scale,
            metric_where = metric_where(def),
        ),
    };
    CompiledQuery { sql, params }
}

fn compile_timeseries_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    bucket: Bucket,
    dimensions: &[String],
) -> CompiledQuery {
    let mut params = metric_params(def, req);
    params.extend(req.entity_ids.iter().cloned());
    let entities = placeholders(req.entity_ids.len());
    let bucket = bucket_expr(bucket);
    let (dim_select, dim_group) = dimension_select_group(dimensions);
    let group = if dim_group.is_empty() {
        "entity_id, bucket_start".to_owned()
    } else {
        format!("entity_id, bucket_start, {dim_group}")
    };
    let observation_table = observation_table(def.observation_source());
    let limit = query_row_limit();
    let sql = match &def.spec {
        ComputationSpec::Sum { .. } => format!(
            r"
            SELECT
                entity_id,
                toString({bucket}) AS bucket_start{dim_select},
                sumIf(value, value IS NOT NULL) AS value
            FROM {observation_table}
            WHERE {metric_where}
              AND entity_id IN ({entities})
            GROUP BY {group}
            ORDER BY entity_id, bucket_start
            LIMIT {limit}
            ",
            metric_where = metric_where(def),
        ),
        ComputationSpec::Ratio { scale, .. } => format!(
            r"
            SELECT
                entity_id,
                toString({bucket}) AS bucket_start{dim_select},
                {scale} * sumIf(value, measure_key = ? AND value IS NOT NULL)
                    / nullIf(sumIf(value, measure_key = ? AND value IS NOT NULL), 0) AS value
            FROM {observation_table}
            WHERE {metric_where}
              AND entity_id IN ({entities})
            GROUP BY {group}
            ORDER BY entity_id, bucket_start
            LIMIT {limit}
            ",
            metric_where = metric_where(def),
            scale = scale,
        ),
    };
    CompiledQuery { sql, params }
}

fn compile_breakdown_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    dimensions: &[String],
) -> CompiledQuery {
    let mut params = metric_params(def, req);
    params.extend(req.entity_ids.iter().cloned());
    let entities = placeholders(req.entity_ids.len());
    let (dim_select, dim_group) = dimension_select_group(dimensions);
    let group = if dim_group.is_empty() {
        "entity_id".to_owned()
    } else {
        format!("entity_id, {dim_group}")
    };
    let observation_table = observation_table(def.observation_source());
    let limit = query_row_limit();
    let sql = match &def.spec {
        ComputationSpec::Sum { .. } => format!(
            r"
            SELECT
                entity_id{dim_select},
                sumIf(value, value IS NOT NULL) AS value
            FROM {observation_table}
            WHERE {metric_where}
              AND entity_id IN ({entities})
            GROUP BY {group}
            ORDER BY entity_id
            LIMIT {limit}
            ",
            metric_where = metric_where(def),
        ),
        ComputationSpec::Ratio { scale, .. } => format!(
            r"
            SELECT
                entity_id{dim_select},
                {scale} * sumIf(value, measure_key = ? AND value IS NOT NULL)
                    / nullIf(sumIf(value, measure_key = ? AND value IS NOT NULL), 0) AS value
            FROM {observation_table}
            WHERE {metric_where}
              AND entity_id IN ({entities})
            GROUP BY {group}
            ORDER BY entity_id
            LIMIT {limit}
            ",
            metric_where = metric_where(def),
            scale = scale,
        ),
    };
    CompiledQuery { sql, params }
}

fn compile_peer_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    cohort_key: &str,
) -> CompiledQuery {
    let mut params = Vec::new();
    params.push(req.entity_type.clone());
    params.push(cohort_key.to_owned());
    params.extend(req.entity_ids.iter().cloned());
    params.push(req.entity_type.clone());
    params.push(cohort_key.to_owned());
    params.extend(metric_params(def, req));

    let entities = placeholders(req.entity_ids.len());
    let observation_table = observation_table(def.observation_source());
    let cohort_table = cohort_table(CohortSource::MetricEntityCohortsCurrent);
    let metric_value = match &def.spec {
        ComputationSpec::Sum { .. } => "sumIf(value, value IS NOT NULL)".to_owned(),
        ComputationSpec::Ratio { scale, .. } => format!(
            "{scale} * sumIf(value, measure_key = ? AND value IS NOT NULL) / nullIf(sumIf(value, measure_key = ? AND value IS NOT NULL), 0)"
        ),
    };
    let limit = query_row_limit();
    let sql = format!(
        r"
        WITH
        targets AS (
            SELECT DISTINCT
                entity_id,
                cohort_id
            FROM {cohort_table}
            WHERE entity_type = ?
              AND cohort_key = ?
              AND entity_id IN ({entities})
              AND cohort_id IS NOT NULL
        ),
        cohort AS (
            SELECT DISTINCT
                entity_id,
                cohort_id
            FROM {cohort_table}
            WHERE entity_type = ?
              AND cohort_key = ?
              AND cohort_id IN (SELECT cohort_id FROM targets)
        ),
        metric_values AS (
            SELECT
                entity_id,
                {metric_value} AS value
            FROM {observation_table}
            WHERE {metric_where}
            GROUP BY entity_id
        ),
        entity_values AS (
            SELECT
                cohort.entity_id AS entity_id,
                cohort.cohort_id AS cohort_id,
                metric_values.value AS value
            FROM cohort
            LEFT JOIN metric_values
                ON metric_values.entity_id = cohort.entity_id
        ),
        peers AS (
            SELECT
                cohort_id,
                entity_id,
                value
            FROM entity_values
            WHERE value IS NOT NULL
        )
        SELECT
            targets.entity_id AS entity_id,
            target_values.value AS target_value,
            if(uniqExact(peers.entity_id) >= {min_peer_n}, toNullable(quantileExact(0.25)(peers.value)), NULL) AS p25,
            if(uniqExact(peers.entity_id) >= {min_peer_n}, toNullable(quantileExact(0.5)(peers.value)), NULL) AS median,
            if(uniqExact(peers.entity_id) >= {min_peer_n}, toNullable(quantileExact(0.75)(peers.value)), NULL) AS p75,
            if(uniqExact(peers.entity_id) >= {min_peer_n}, toNullable(min(peers.value)), NULL) AS min,
            if(uniqExact(peers.entity_id) >= {min_peer_n}, toNullable(max(peers.value)), NULL) AS max,
            toUInt64(uniqExact(peers.entity_id)) AS n
        FROM targets
        LEFT JOIN entity_values AS target_values
            ON target_values.entity_id = targets.entity_id
        LEFT JOIN peers
            ON peers.cohort_id = targets.cohort_id
        GROUP BY targets.entity_id, target_values.value
        LIMIT {limit}
        SETTINGS join_use_nulls = 1
        ",
        metric_where = metric_where(def),
        min_peer_n = MIN_PEER_N,
    );
    CompiledQuery { sql, params }
}

// No tenant_id predicate: warehouse tenant isolation is not implemented
// platform-wide (the legacy query engine also queries without it), and the
// control-plane tenant UUID has no defined mapping to the warehouse
// tenant_id strings stamped at ingestion. The observation and cohort
// contracts keep the tenant_id column so isolation can be added here in one
// place once the platform defines that mapping.
fn metric_where(def: &MetricDefinition) -> &'static str {
    match &def.spec {
        ComputationSpec::Sum { .. } => {
            "source_key = ? AND entity_type = ? AND metric_date >= toDate(?) AND metric_date <= toDate(?) AND measure_key = ?"
        }
        ComputationSpec::Ratio { .. } => {
            "source_key = ? AND entity_type = ? AND metric_date >= toDate(?) AND metric_date <= toDate(?) AND measure_key IN (?, ?)"
        }
    }
}

fn metric_params(def: &MetricDefinition, req: &ValidatedMetricResultsRequest) -> Vec<String> {
    match &def.spec {
        ComputationSpec::Sum { value } => vec![
            value.source_key.clone(),
            req.entity_type.clone(),
            req.from.to_string(),
            req.to.to_string(),
            value.measure_key.clone(),
        ],
        ComputationSpec::Ratio {
            numerator,
            denominator,
            ..
        } => {
            let mut params = vec![
                numerator.measure_key.clone(),
                denominator.measure_key.clone(),
            ];
            params.extend([
                numerator.source_key.clone(),
                req.entity_type.clone(),
                req.from.to_string(),
                req.to.to_string(),
                numerator.measure_key.clone(),
                denominator.measure_key.clone(),
            ]);
            params
        }
    }
}

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}

fn bucket_expr(bucket: Bucket) -> &'static str {
    match bucket {
        Bucket::Day => "metric_date",
        Bucket::Week => "toStartOfWeek(metric_date, 1)",
        Bucket::Month => "toStartOfMonth(metric_date)",
    }
}

fn observation_table(source: ObservationSource) -> &'static str {
    match source {
        ObservationSource::AiMetricObservations => "insight.ai_metric_observations",
    }
}

fn cohort_table(source: CohortSource) -> &'static str {
    match source {
        CohortSource::MetricEntityCohortsCurrent => "insight.metric_entity_cohorts_current",
    }
}

pub(crate) fn dimension_aliases(idx: usize) -> (String, String) {
    (format!("dim_{idx}_value"), format!("dim_{idx}_label"))
}

fn dimension_select_group(dimensions: &[String]) -> (String, String) {
    let mut select = String::new();
    let mut groups = Vec::with_capacity(dimensions.len() * 2);
    for (idx, dimension) in dimensions.iter().enumerate() {
        let (value_alias, label_alias) = dimension_aliases(idx);
        let _ = write!(
            select,
            ", {value} AS {value_alias}, {label} AS {label_alias}",
            value = dimension_value_expr(dimension),
            label = dimension_label_expr(dimension)
        );
        groups.push(value_alias);
        groups.push(label_alias);
    }
    (select, groups.join(", "))
}

fn dimension_value_expr(dimension: &str) -> String {
    format!(
        r"
        if(
            length(arrayFilter(d -> tupleElement(d, 1) = '{dimension}', dimensions)) = 0,
            '{UNKNOWN_DIMENSION_VALUE}',
            coalesce(
                tupleElement(arrayFilter(d -> tupleElement(d, 1) = '{dimension}', dimensions)[1], 2),
                '{UNKNOWN_DIMENSION_VALUE}'
            )
        )
        "
    )
}

fn dimension_label_expr(dimension: &str) -> String {
    format!(
        r"
        if(
            length(arrayFilter(d -> tupleElement(d, 1) = '{dimension}', dimensions)) = 0,
            '{UNKNOWN_DIMENSION_LABEL}',
            coalesce(
                tupleElement(arrayFilter(d -> tupleElement(d, 1) = '{dimension}', dimensions)[1], 3),
                '{UNKNOWN_DIMENSION_LABEL}'
            )
        )
        "
    )
}

fn optional_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom("expected unsigned integer"))
            .map(Some),
        Some(serde_json::Value::String(value)) => value
            .parse::<u64>()
            .map(Some)
            .map_err(serde::de::Error::custom),
        Some(_) => Err(serde::de::Error::custom("expected unsigned integer")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    use crate::domain::metric_definitions::definition::{
        MetricBase, MetricDirection, MetricFormat, MetricInput, MetricInputRole,
    };

    fn base(dimensions: Vec<&str>) -> MetricBase {
        MetricBase {
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
        }
    }

    fn input(role: MetricInputRole, measure_key: &str) -> MetricInput {
        MetricInput {
            role,
            observation_source: ObservationSource::AiMetricObservations,
            source_key: "ai_usage".to_owned(),
            measure_key: measure_key.to_owned(),
        }
    }

    fn sum_metric() -> MetricDefinition {
        MetricDefinition {
            base: base(vec!["tool"]),
            spec: ComputationSpec::Sum {
                value: input(MetricInputRole::Value, "accepted_lines"),
            },
        }
    }

    fn ratio_metric() -> MetricDefinition {
        MetricDefinition {
            base: base(vec!["tool"]),
            spec: ComputationSpec::Ratio {
                numerator: input(MetricInputRole::Numerator, "accepted_edit_actions"),
                denominator: input(MetricInputRole::Denominator, "tool_use_offered"),
                scale: 100.0,
            },
        }
    }

    fn request() -> ValidatedMetricResultsRequest {
        ValidatedMetricResultsRequest {
            entity_type: "person".to_owned(),
            entity_ids: vec!["a@x.io".to_owned(), "b@x.io".to_owned()],
            from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap_or_default(),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap_or_default(),
            metrics: Vec::new(),
        }
    }

    #[test]
    fn sum_period_query_binds_scope_then_entities() {
        let query = compile_view_query(&sum_metric(), &request(), &ValidatedMetricView::Period);
        assert!(query.sql.contains("FROM insight.ai_metric_observations"));
        assert!(!query.sql.contains("tenant_id"));
        assert!(query.sql.contains("measure_key = ?"));
        assert!(query.sql.contains("GROUP BY entity_id"));
        assert_eq!(
            query.params,
            vec![
                "ai_usage",
                "person",
                "2026-01-01",
                "2026-01-31",
                "accepted_lines",
                "a@x.io",
                "b@x.io",
            ]
        );
    }

    #[test]
    fn ratio_period_query_binds_select_measures_first() {
        let query = compile_view_query(&ratio_metric(), &request(), &ValidatedMetricView::Period);
        assert!(query.sql.contains("nullIf"));
        assert!(query.sql.contains("100 *"));
        assert!(query.sql.contains("measure_key IN (?, ?)"));
        assert_eq!(
            query.params,
            vec![
                "accepted_edit_actions",
                "tool_use_offered",
                "ai_usage",
                "person",
                "2026-01-01",
                "2026-01-31",
                "accepted_edit_actions",
                "tool_use_offered",
                "a@x.io",
                "b@x.io",
            ]
        );
    }

    #[test]
    fn timeseries_query_uses_bucket_expression() {
        for (bucket, expr) in [
            (Bucket::Day, "metric_date"),
            (Bucket::Week, "toStartOfWeek(metric_date, 1)"),
            (Bucket::Month, "toStartOfMonth(metric_date)"),
        ] {
            let query = compile_view_query(
                &sum_metric(),
                &request(),
                &ValidatedMetricView::Timeseries {
                    bucket,
                    dimensions: vec![],
                },
            );
            assert!(
                query
                    .sql
                    .contains(&format!("toString({expr}) AS bucket_start"))
            );
            assert!(query.sql.contains("GROUP BY entity_id, bucket_start"));
        }
    }

    #[test]
    fn dimensioned_query_emits_value_and_label_aliases() {
        let query = compile_view_query(
            &sum_metric(),
            &request(),
            &ValidatedMetricView::Breakdown {
                dimensions: vec!["tool".to_owned()],
            },
        );
        assert!(query.sql.contains("AS dim_0_value"));
        assert!(query.sql.contains("AS dim_0_label"));
        assert!(query.sql.contains("tupleElement(d, 1) = 'tool'"));
        assert!(
            query
                .sql
                .contains("GROUP BY entity_id, dim_0_value, dim_0_label")
        );
    }

    #[test]
    fn peer_query_binds_cohort_scopes_then_metric_scope() {
        let query = compile_view_query(
            &sum_metric(),
            &request(),
            &ValidatedMetricView::Peer {
                cohort_key: "org_unit".to_owned(),
            },
        );
        assert!(
            query
                .sql
                .contains("FROM insight.metric_entity_cohorts_current")
        );
        assert!(query.sql.contains("WHERE value IS NOT NULL"));
        assert!(!query.sql.contains("AND peer.value IS NOT NULL"));
        assert_eq!(
            query.params,
            vec![
                "person",
                "org_unit",
                "a@x.io",
                "b@x.io",
                "person",
                "org_unit",
                "ai_usage",
                "person",
                "2026-01-01",
                "2026-01-31",
                "accepted_lines",
            ]
        );
    }

    #[test]
    fn peer_queries_never_fabricate_zero_observations() {
        // Honest-null through the runtime: cohort members without observed
        // values stay NULL and drop out of the peer pool — absence of rows
        // cannot be distinguished from "not covered by the source", so the
        // peer query must not invent zeros for them.
        for def in [sum_metric(), ratio_metric()] {
            let query = compile_view_query(
                &def,
                &request(),
                &ValidatedMetricView::Peer {
                    cohort_key: "org_unit".to_owned(),
                },
            );
            assert!(query.sql.contains("metric_values.value AS value"));
            assert!(!query.sql.contains("coalesce(metric_values.value, 0)"));
        }
    }

    #[test]
    fn peer_queries_suppress_percentiles_below_min_pool_size() {
        for def in [sum_metric(), ratio_metric()] {
            let query = compile_view_query(
                &def,
                &request(),
                &ValidatedMetricView::Peer {
                    cohort_key: "org_unit".to_owned(),
                },
            );
            let guard = format!("uniqExact(peers.entity_id) >= {MIN_PEER_N}");
            assert_eq!(
                query.sql.matches(&guard).count(),
                5,
                "every percentile/min/max must carry the disclosure guard"
            );
            assert!(
                query
                    .sql
                    .contains("toUInt64(uniqExact(peers.entity_id)) AS n")
            );
            // Duplicate cohort membership must not fan out the pool.
            assert_eq!(query.sql.matches("SELECT DISTINCT").count(), 2);
            // Honest-null must not depend on server config or column typing.
            assert!(query.sql.contains("SETTINGS join_use_nulls = 1"));
        }
    }

    #[test]
    fn queries_carry_row_limit() {
        let query = compile_view_query(&sum_metric(), &request(), &ValidatedMetricView::Period);
        assert!(query.sql.contains(&format!("LIMIT {}", query_row_limit())));
    }
}
