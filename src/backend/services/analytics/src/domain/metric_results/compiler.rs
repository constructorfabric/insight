use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;

use serde::Deserialize;

use super::batch::{peer_aliases, period_alias};
use super::validation::{HISTOGRAM_BINS, ValidatedMetricResultsRequest, query_row_limit};
use super::view::Bucket;
use crate::domain::metric_definitions::{
    CohortSource, ComputationSpec, MetricDefinition, ObservationRelation,
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

/// One observed (entity, bin) pair plus the entity's exact value bounds.
/// The SQL owns bin membership only; the builder derives all bin edges from
/// the bounds so displayed edges of empty and observed bins cannot drift.
#[derive(Debug, Deserialize)]
pub struct HistogramQueryRow {
    pub entity_id: String,
    pub bin_idx: u32,
    pub entity_lo: f64,
    pub entity_hi: f64,
    #[serde(default, deserialize_with = "optional_u64")]
    pub bin_count: Option<u64>,
}

pub(crate) fn compile_period_batch_query(
    defs: &[&MetricDefinition],
    req: &ValidatedMetricResultsRequest,
) -> CompiledQuery {
    let mut params = Vec::new();
    let selects = item_value_selects(defs, &mut params, period_alias);
    let metric_scope = shared_observation_where(defs, req, &mut params);
    params.extend(req.entity_ids.iter().cloned());
    let entities = placeholders(req.entity_ids.len());
    let observation_table = batch_observation_table(defs);
    let limit = query_row_limit();
    let sql = format!(
        r"
        SELECT
            entity_id{selects}
        FROM {observation_table}
        WHERE {metric_scope}
          AND entity_id IN ({entities})
        GROUP BY entity_id
        LIMIT {limit}
        "
    );
    CompiledQuery { sql, params }
}

pub(crate) fn compile_timeseries_query(
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
    let observation_table = observation_table(def.observation_relation());
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
        ComputationSpec::Median { .. } => format!(
            r"
            SELECT
                entity_id,
                toString({bucket}) AS bucket_start{dim_select},
                quantileExactIf(0.5)(value, value IS NOT NULL) AS value
            FROM {observation_table}
            WHERE {metric_where}
              AND entity_id IN ({entities})
            GROUP BY {group}
            ORDER BY entity_id, bucket_start
            LIMIT {limit}
            ",
            metric_where = metric_where(def),
        ),
    };
    CompiledQuery { sql, params }
}

pub(crate) fn compile_breakdown_query(
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
    let observation_table = observation_table(def.observation_relation());
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
        ComputationSpec::Median { .. } => format!(
            r"
            SELECT
                entity_id{dim_select},
                quantileExactIf(0.5)(value, value IS NOT NULL) AS value
            FROM {observation_table}
            WHERE {metric_where}
              AND entity_id IN ({entities})
            GROUP BY {group}
            ORDER BY entity_id
            LIMIT {limit}
            ",
            metric_where = metric_where(def),
        ),
    };
    CompiledQuery { sql, params }
}

// Deterministic fixed-width binning over each entity's exact [min, max]:
// pure arithmetic over exact aggregates, so identical data always yields
// identical bins (the adaptive `histogram()` aggregate is merge-order
// dependent). `least(max_bin, …)` closes the last bin at the maximum; a
// degenerate range (all values identical) maps everything to bin 0, which
// the builder renders as one [v, v] bin. Validation guarantees the metric is
// a median (single-measure predicate), so metric_where/metric_params fit.
pub(crate) fn compile_histogram_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
) -> CompiledQuery {
    let mut params = metric_params(def, req);
    params.extend(req.entity_ids.iter().cloned());
    let entities = placeholders(req.entity_ids.len());
    let observation_table = observation_table(def.observation_relation());
    let bins = HISTOGRAM_BINS;
    let max_bin = HISTOGRAM_BINS - 1;
    let limit = query_row_limit();
    let sql = format!(
        r"
        WITH
        events AS (
            SELECT
                entity_id,
                assumeNotNull(value) AS value
            FROM {observation_table}
            WHERE {metric_where}
              AND entity_id IN ({entities})
              AND value IS NOT NULL
        ),
        bounds AS (
            SELECT
                entity_id,
                min(value) AS entity_lo,
                max(value) AS entity_hi
            FROM events
            GROUP BY entity_id
        )
        SELECT
            events.entity_id AS entity_id,
            if(
                bounds.entity_hi = bounds.entity_lo,
                0,
                toUInt32(least({max_bin}, toInt64(floor(
                    (events.value - bounds.entity_lo) * {bins} / (bounds.entity_hi - bounds.entity_lo)
                ))))
            ) AS bin_idx,
            any(bounds.entity_lo) AS entity_lo,
            any(bounds.entity_hi) AS entity_hi,
            toUInt64(count()) AS bin_count
        FROM events
        INNER JOIN bounds ON bounds.entity_id = events.entity_id
        GROUP BY entity_id, bin_idx
        ORDER BY entity_id, bin_idx
        LIMIT {limit}
        ",
        metric_where = metric_where(def),
    );
    CompiledQuery { sql, params }
}

// The cohort join shape relies on the gold contract that a person has at
// most one cohort row per (entity_type, cohort_key): the model ends in
// `LIMIT 1 BY tenant_id, entity_id` and assert_metric_entity_cohorts_unique
// asserts it at every dbt build. If that contract ever loosened (multi-cohort
// membership), the GROUP BY below would blend pools and double-weight shared
// peers — a state the dbt test exists to catch loudly; no SQL hardening here.
pub(crate) fn compile_peer_batch_query(
    defs: &[&MetricDefinition],
    req: &ValidatedMetricResultsRequest,
    cohort_key: &str,
) -> CompiledQuery {
    let mut params = Vec::new();
    params.push(req.entity_type.clone());
    params.push(cohort_key.to_owned());
    params.extend(req.entity_ids.iter().cloned());
    params.push(req.entity_type.clone());
    params.push(cohort_key.to_owned());
    let value_selects = item_value_selects(defs, &mut params, period_alias);
    let metric_scope = shared_observation_where(defs, req, &mut params);

    let entities = placeholders(req.entity_ids.len());
    let observation_table = batch_observation_table(defs);
    let cohort_table = cohort_table(CohortSource::MetricEntityCohortsCurrent);
    let limit = query_row_limit();

    let mut carried = String::new();
    let mut stats_selects = String::new();
    let mut target_group = String::new();
    for (item_index, _) in defs.iter().enumerate() {
        let value = period_alias(item_index);
        let aliases = peer_aliases(item_index);
        let _ = write!(
            carried,
            ",
                metric_values.{value} AS {value}"
        );
        let observed = format!("peer.{value} IS NOT NULL");
        let pool = format!("uniqExactIf(peer.entity_id, {observed})");
        let _ = write!(
            stats_selects,
            ",
            target_values.{value} AS {target},
            if({pool} >= {min_peer_n}, toNullable(quantileExactIf(0.25)(peer.{value}, {observed})), NULL) AS {p25},
            if({pool} >= {min_peer_n}, toNullable(quantileExactIf(0.5)(peer.{value}, {observed})), NULL) AS {median},
            if({pool} >= {min_peer_n}, toNullable(quantileExactIf(0.75)(peer.{value}, {observed})), NULL) AS {p75},
            if({pool} >= {min_peer_n}, toNullable(minIfOrNull(peer.{value}, {observed})), NULL) AS {min},
            if({pool} >= {min_peer_n}, toNullable(maxIfOrNull(peer.{value}, {observed})), NULL) AS {max},
            toUInt64({pool}) AS {n}",
            target = aliases.target,
            p25 = aliases.p25,
            median = aliases.median,
            p75 = aliases.p75,
            min = aliases.min,
            max = aliases.max,
            n = aliases.n,
            min_peer_n = MIN_PEER_N,
        );
        let _ = write!(target_group, ", target_values.{value}");
    }

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
                entity_id{value_selects}
            FROM {observation_table}
            WHERE {metric_scope}
            GROUP BY entity_id
        ),
        entity_values AS (
            SELECT
                cohort.entity_id AS entity_id,
                cohort.cohort_id AS cohort_id{carried}
            FROM cohort
            LEFT JOIN metric_values
                ON metric_values.entity_id = cohort.entity_id
        )
        SELECT
            targets.entity_id AS entity_id{stats_selects}
        FROM targets
        LEFT JOIN entity_values AS target_values
            ON target_values.entity_id = targets.entity_id
        LEFT JOIN entity_values AS peer
            ON peer.cohort_id = targets.cohort_id
        GROUP BY targets.entity_id{target_group}
        LIMIT {limit}
        SETTINGS join_use_nulls = 1
        "
    );
    CompiledQuery { sql, params }
}

fn item_value_selects(
    defs: &[&MetricDefinition],
    params: &mut Vec<String>,
    alias: fn(usize) -> String,
) -> String {
    let mut selects = String::new();
    for (item_index, def) in defs.iter().enumerate() {
        let expr = item_value_expr(def, params);
        let _ = write!(
            selects,
            ",
                {expr} AS {alias}",
            alias = alias(item_index)
        );
    }
    selects
}

// sumIfOrNull, not sumIf: a plain sumIf yields 0 when the item matches no
// rows of an entity that has rows for other items, fabricating an
// observation the peer pool must not see. OrNull pins NULL-on-no-match.
// (Today an all-NULL-values entity row set cannot occur — the observation
// macro guards HAVING countIf(value IS NOT NULL) > 0 — but a future custom
// SQL source could produce one; OrNull excludes it from pools by
// construction.)
fn item_value_expr(def: &MetricDefinition, params: &mut Vec<String>) -> String {
    match &def.spec {
        ComputationSpec::Sum { value } => {
            params.push(value.source_key.clone());
            params.push(value.measure_key.clone());
            "sumIfOrNull(value, source_key = ? AND measure_key = ? AND value IS NOT NULL)"
                .to_owned()
        }
        ComputationSpec::Ratio {
            numerator,
            denominator,
            scale,
        } => {
            // Ratio inputs share one source (enforced at definition load:
            // "ratio inputs must share one source"), so the numerator's
            // source_key scopes both halves.
            params.push(numerator.source_key.clone());
            params.push(numerator.measure_key.clone());
            params.push(numerator.source_key.clone());
            params.push(denominator.measure_key.clone());
            format!(
                "{scale} * sumIf(value, source_key = ? AND measure_key = ? AND value IS NOT NULL) / nullIf(sumIf(value, source_key = ? AND measure_key = ? AND value IS NOT NULL), 0)"
            )
        }
        ComputationSpec::Median { value } => {
            // OrNull so an entity present in the batch (via another measure)
            // but with no rows for this measure comes back NULL, not 0 — the
            // builder never zero-fills medians (honest-null).
            params.push(value.source_key.clone());
            params.push(value.measure_key.clone());
            "quantileExactIfOrNull(0.5)(value, source_key = ? AND measure_key = ? AND value IS NOT NULL)"
                .to_owned()
        }
    }
}

fn shared_observation_where(
    defs: &[&MetricDefinition],
    req: &ValidatedMetricResultsRequest,
    params: &mut Vec<String>,
) -> String {
    params.push(req.entity_type.clone());
    params.push(req.from.to_string());
    params.push(req.to.to_string());
    let pairs = measure_pairs(defs);
    for (source_key, measure_key) in &pairs {
        params.push(source_key.clone());
        params.push(measure_key.clone());
    }
    let pair_placeholders = vec!["(?, ?)"; pairs.len()].join(", ");
    format!(
        "entity_type = ? AND metric_date >= toDate(?) AND metric_date <= toDate(?) AND (source_key, measure_key) IN ({pair_placeholders})"
    )
}

fn measure_pairs(defs: &[&MetricDefinition]) -> BTreeSet<(String, String)> {
    defs.iter()
        .flat_map(|def| match &def.spec {
            ComputationSpec::Sum { value } | ComputationSpec::Median { value } => {
                vec![(value.source_key.clone(), value.measure_key.clone())]
            }
            ComputationSpec::Ratio {
                numerator,
                denominator,
                ..
            } => vec![
                (numerator.source_key.clone(), numerator.measure_key.clone()),
                (
                    numerator.source_key.clone(),
                    denominator.measure_key.clone(),
                ),
            ],
        })
        .collect()
}

fn batch_observation_table(defs: &[&MetricDefinition]) -> String {
    let def = defs
        .first()
        .unwrap_or_else(|| unreachable!("batches are planned from at least one metric view"));
    observation_table(def.observation_relation())
}

// No tenant_id predicate: warehouse tenant isolation is not implemented
// platform-wide (the legacy query engine also queries without it), and the
// control-plane tenant UUID has no defined mapping to the warehouse
// tenant_id strings stamped at ingestion. The observation and cohort
// contracts keep the tenant_id column so isolation can be added here in one
// place once the platform defines that mapping.
fn metric_where(def: &MetricDefinition) -> &'static str {
    match &def.spec {
        ComputationSpec::Sum { .. } | ComputationSpec::Median { .. } => {
            "source_key = ? AND entity_type = ? AND metric_date >= toDate(?) AND metric_date <= toDate(?) AND measure_key = ?"
        }
        ComputationSpec::Ratio { .. } => {
            "source_key = ? AND entity_type = ? AND metric_date >= toDate(?) AND metric_date <= toDate(?) AND measure_key IN (?, ?)"
        }
    }
}

fn metric_params(def: &MetricDefinition, req: &ValidatedMetricResultsRequest) -> Vec<String> {
    match &def.spec {
        ComputationSpec::Sum { value } | ComputationSpec::Median { value } => vec![
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

fn observation_table(relation: &ObservationRelation) -> String {
    let (database, table) = relation.table_ref();
    format!("{database}.{table}")
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
            observation_relation: ObservationRelation::parse("ai_metric_observations")
                .unwrap_or_else(|| panic!("fixture relation must parse")),
            source_key: "ai_usage".to_owned(),
            measure_key: measure_key.to_owned(),
        }
    }

    fn median_metric() -> MetricDefinition {
        MetricDefinition {
            base: base(vec!["source"]),
            spec: ComputationSpec::Median {
                value: input(MetricInputRole::Value, "pr_cycle_hours"),
            },
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
    fn period_batch_binds_item_params_then_scope_then_pairs_then_entities() {
        let (sum, ratio) = (sum_metric(), ratio_metric());
        let query = compile_period_batch_query(&[&sum, &ratio], &request());
        assert!(query.sql.contains("FROM insight.ai_metric_observations"));
        assert!(!query.sql.contains("tenant_id"));
        assert!(query.sql.contains("AS m0"));
        assert!(query.sql.contains("AS m1"));
        assert!(
            query
                .sql
                .contains("sumIfOrNull(value, source_key = ? AND measure_key = ?")
        );
        assert!(query.sql.contains("nullIf"));
        assert!(query.sql.contains("100 *"));
        assert!(
            query
                .sql
                .contains("(source_key, measure_key) IN ((?, ?), (?, ?), (?, ?))")
        );
        assert!(query.sql.contains("GROUP BY entity_id"));
        assert_eq!(
            query.params,
            vec![
                // item exprs, batch order
                "ai_usage",
                "accepted_lines",
                "ai_usage",
                "accepted_edit_actions",
                "ai_usage",
                "tool_use_offered",
                // shared scope
                "person",
                "2026-01-01",
                "2026-01-31",
                // deduped (source_key, measure_key) pairs, BTreeSet order
                "ai_usage",
                "accepted_edit_actions",
                "ai_usage",
                "accepted_lines",
                "ai_usage",
                "tool_use_offered",
                // entities
                "a@x.io",
                "b@x.io",
            ]
        );
    }

    #[test]
    fn period_batch_of_one_uses_wide_aliases() {
        let sum = sum_metric();
        let query = compile_period_batch_query(&[&sum], &request());
        assert!(query.sql.contains("AS m0"));
        assert!(!query.sql.contains("AS value"));
    }

    #[test]
    fn ratio_item_binds_numerator_source_for_both_halves() {
        let ratio = ratio_metric();
        let query = compile_period_batch_query(&[&ratio], &request());
        // Ratio inputs share one source by the definition-load invariant;
        // both sumIf halves and the pruning pair carry the numerator's key.
        assert_eq!(
            query.params,
            vec![
                "ai_usage",
                "accepted_edit_actions",
                "ai_usage",
                "tool_use_offered",
                "person",
                "2026-01-01",
                "2026-01-31",
                "ai_usage",
                "accepted_edit_actions",
                "ai_usage",
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
            let query = compile_timeseries_query(&sum_metric(), &request(), bucket, &[]);
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
        let query = compile_breakdown_query(&sum_metric(), &request(), &["tool".to_owned()]);
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
    fn peer_batch_keeps_cohort_ctes_and_param_order() {
        let sum = sum_metric();
        let query = compile_peer_batch_query(&[&sum], &request(), "org_unit");
        assert!(
            query
                .sql
                .contains("FROM insight.metric_entity_cohorts_current")
        );
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
                "accepted_lines",
                "person",
                "2026-01-01",
                "2026-01-31",
                "ai_usage",
                "accepted_lines",
            ]
        );
    }

    #[test]
    fn peer_batch_never_fabricates_zero_observations() {
        // Honest-null through the runtime: cohort members without observed
        // values stay NULL and drop out of the pool per metric — absence of
        // rows cannot be distinguished from "not covered by the source", so
        // the peer query must not invent zeros for them.
        let (sum, ratio) = (sum_metric(), ratio_metric());
        let query = compile_peer_batch_query(&[&sum, &ratio], &request(), "org_unit");
        assert!(query.sql.contains("sumIfOrNull"));
        assert!(!query.sql.contains("coalesce"));
        assert!(query.sql.contains("metric_values.m0 AS m0"));
    }

    #[test]
    fn peer_batch_guards_every_percentile_per_item() {
        let (sum, ratio) = (sum_metric(), ratio_metric());
        let query = compile_peer_batch_query(&[&sum, &ratio], &request(), "org_unit");
        for item in 0..2 {
            let guard =
                format!("uniqExactIf(peer.entity_id, peer.m{item} IS NOT NULL) >= {MIN_PEER_N}");
            assert_eq!(
                query.sql.matches(&guard).count(),
                5,
                "every percentile/min/max must carry the per-item disclosure guard"
            );
            assert!(query.sql.contains(&format!(
                "toUInt64(uniqExactIf(peer.entity_id, peer.m{item} IS NOT NULL)) AS m{item}_n"
            )));
            assert!(query.sql.contains(&format!("AS m{item}_target")));
        }
        // Duplicate cohort membership must not fan out the pool.
        assert_eq!(query.sql.matches("SELECT DISTINCT").count(), 2);
        // Honest-null must not depend on server config or column typing.
        assert!(query.sql.contains("SETTINGS join_use_nulls = 1"));
        assert!(
            query
                .sql
                .contains("GROUP BY targets.entity_id, target_values.m0, target_values.m1")
        );
    }

    #[test]
    fn queries_carry_row_limit() {
        let (sum, ratio) = (sum_metric(), ratio_metric());
        let limit = format!("LIMIT {}", query_row_limit());
        assert!(
            compile_period_batch_query(&[&sum], &request())
                .sql
                .contains(&limit)
        );
        assert!(
            compile_peer_batch_query(&[&ratio], &request(), "org_unit")
                .sql
                .contains(&limit)
        );
    }

    #[test]
    fn batched_placeholder_count_matches_params() {
        // Params are emitted in lockstep with SQL fragments; a drift between
        // `?` order and the param vector silently binds wrong values. The mix
        // interleaves a median column (2 params) between sum (2) and ratio
        // (4) — the real git batch shape — so a per-computation param/`?`
        // desync surfaces here, not just in single-computation batches.
        let (sum, median, ratio) = (sum_metric(), median_metric(), ratio_metric());
        for query in [
            compile_period_batch_query(&[&sum, &median, &ratio], &request()),
            compile_peer_batch_query(&[&sum, &median, &ratio], &request(), "org_unit"),
        ] {
            assert_eq!(query.sql.matches('?').count(), query.params.len());
        }
    }

    #[test]
    fn median_batches_as_a_quantile_ornull_column() {
        // A median metric joins the period/peer batch as one wide column.
        // OrNull so an entity present via another measure but with no rows
        // for this one comes back NULL, not 0 (the builder never zero-fills
        // medians). Placeholder/param lockstep still holds.
        for query in [
            compile_period_batch_query(&[&median_metric()], &request()),
            compile_peer_batch_query(&[&median_metric()], &request(), "org_unit"),
        ] {
            assert!(
                query.sql.contains(
                    "quantileExactIfOrNull(0.5)(value, source_key = ? AND measure_key = ?"
                ),
                "median must batch as an OrNull quantile column"
            );
            assert_eq!(query.sql.matches('?').count(), query.params.len());
        }
    }

    #[test]
    fn median_single_views_use_exact_median() {
        let ts = compile_timeseries_query(&median_metric(), &request(), Bucket::Week, &[]);
        assert!(
            ts.sql
                .contains("quantileExactIf(0.5)(value, value IS NOT NULL)")
        );
        assert!(ts.sql.contains("GROUP BY entity_id, bucket_start"));
        let bd = compile_breakdown_query(&median_metric(), &request(), &["source".to_owned()]);
        assert!(
            bd.sql
                .contains("quantileExactIf(0.5)(value, value IS NOT NULL)")
        );
    }

    #[test]
    fn histogram_query_bins_deterministically_from_entity_bounds() {
        let query = compile_histogram_query(&median_metric(), &request());
        assert!(query.sql.contains("min(value) AS entity_lo"));
        assert!(query.sql.contains("max(value) AS entity_hi"));
        assert!(query.sql.contains("least(9,"));
        assert!(query.sql.contains("* 10 /"));
        assert!(query.sql.contains("GROUP BY entity_id, bin_idx"));
        assert!(query.sql.contains("bounds.entity_hi = bounds.entity_lo"));
        // Deterministic arithmetic only — never the adaptive aggregate.
        assert!(!query.sql.contains("histogram("));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }
}
