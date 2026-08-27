use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;

use serde::Deserialize;
use uuid::Uuid;

use super::batch::{PeerPopulation, ResolvedGroupLimit, peer_aliases, period_alias};
use super::validation::{
    HISTOGRAM_BINS, ValidatedDimensionFilter, ValidatedEntitySelection,
    ValidatedMetricResultsRequest, query_row_limit,
};
use super::view::Bucket;
use crate::domain::metric_definitions::{
    CohortSource, ComputationSpec, MetricDefinition, ObservationSource, RatioDenominatorAggregation,
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
    pub is_total: u8,
    pub rank: Option<u32>,
    pub remainder: u8,
    pub group_label: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct RankingQueryRow {
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
    #[serde(rename = "link_source_provider")]
    pub source_provider: Option<String>,
    #[serde(rename = "link_source_id")]
    pub source_id: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct RollupQueryRow {
    pub value: Option<f64>,
    #[serde(default, deserialize_with = "optional_u64")]
    pub contributing_entity_count: Option<u64>,
    pub rank: Option<u32>,
    pub remainder: u8,
    pub group_label: Option<String>,
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
    filters: &[ValidatedDimensionFilter],
) -> CompiledQuery {
    let mut params = Vec::new();
    let selects = item_value_selects(defs, &mut params, period_alias);
    let metric_scope = shared_observation_where(defs, req, filters, &mut params);
    let entity_predicate = selected_entity_predicate(req, &mut params);
    let observation_table = batch_observation_table(defs);
    let limit = query_row_limit();
    let inner = format!(
        r"
        SELECT
            entity_id{selects}
        FROM {observation_table}
        WHERE {metric_scope}
          AND {entity_predicate}
        GROUP BY entity_id
        LIMIT {limit}
        "
    );
    let sql = transformed_batch(defs, inner, period_alias);
    CompiledQuery { sql, params }
}

pub(crate) fn compile_timeseries_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    bucket: Bucket,
    dimensions: &[String],
    filters: &[ValidatedDimensionFilter],
    group_limit: Option<&ResolvedGroupLimit>,
) -> CompiledQuery {
    if let Some(group_limit) = group_limit {
        return compile_capped_timeseries_query(def, req, bucket, dimensions, filters, group_limit);
    }
    let mut params = metric_params(def, req);
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_predicate = selected_entity_predicate(req, &mut params);
    let bucket = bucket_expr(bucket);
    let (dim_select, dim_group) = dimension_select_group(dimensions);
    let bucket_group = if dim_group.is_empty() {
        format!("entity_id, {bucket}")
    } else {
        format!("entity_id, {bucket}, {dim_group}")
    };
    let total_group = if dim_group.is_empty() {
        "entity_id".to_owned()
    } else {
        format!("entity_id, {dim_group}")
    };
    let observation_table = observation_table(def.observation_source());
    let limit = query_row_limit();
    let value_expr = grouped_value_expr(def);
    let inner = format!(
        r"
        SELECT
            entity_id,
            toString({bucket}) AS bucket_start{dim_select},
            {value_expr} AS value,
            toUInt8(grouping({bucket})) AS is_total,
            CAST(NULL AS Nullable(UInt32)) AS rank,
            toUInt8(0) AS remainder,
            CAST(NULL AS Nullable(String)) AS group_label
        FROM {observation_table}
        WHERE {metric_where}
          {filter_where}
          AND {entity_predicate}
        GROUP BY GROUPING SETS (({bucket_group}), ({total_group}))
        ORDER BY entity_id, is_total, bucket_start
        LIMIT {limit}
        ",
        metric_where = metric_where(def, req.enforce_tenant_scope),
    );
    let sql = transformed_single(def, inner);
    CompiledQuery { sql, params }
}

pub(crate) fn compile_group_ranking_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    dimensions: &[String],
    filters: &[ValidatedDimensionFilter],
    count: usize,
) -> CompiledQuery {
    let mut params = grouped_value_params(def);
    params.extend(metric_where_params(def, req));
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_predicate = selected_entity_predicate(req, &mut params);
    let (dim_select, dim_group, dim_order) = ranking_dimension_select_group(dimensions);
    let observation_table = observation_table(def.observation_source());
    let value_expr = grouped_value_expr(def);
    let inner = format!(
        r"
        SELECT
            {dim_select},
            {value_expr} AS value
        FROM {observation_table}
        WHERE {metric_where}
          {filter_where}
          AND {entity_predicate}
        GROUP BY {dim_group}
        ",
        metric_where = metric_where(def, req.enforce_tenant_scope),
    );
    let transformed = transformed_single(def, inner);
    let sql = format!(
        r"
        SELECT *
        FROM ({transformed})
        WHERE value IS NOT NULL
        ORDER BY value DESC, {dim_order}
        LIMIT {count}
        "
    );
    CompiledQuery { sql, params }
}

fn compile_capped_timeseries_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    bucket: Bucket,
    dimensions: &[String],
    filters: &[ValidatedDimensionFilter],
    group_limit: &ResolvedGroupLimit,
) -> CompiledQuery {
    let mut params = metric_where_params(def, req);
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_predicate = selected_entity_predicate(req, &mut params);
    let bucket = bucket_expr(bucket);
    let raw_dimensions = dimensions.iter().enumerate().fold(
        String::new(),
        |mut raw_dimensions, (index, dimension)| {
            let _ = write!(
                raw_dimensions,
                ", {} AS raw_dim_{index}",
                dimension_value_expr(dimension)
            );
            raw_dimensions
        },
    );
    let rank_expr = capped_rank_expr(group_limit, dimensions.len(), &mut params);
    params.extend(grouped_value_params(def));
    let dimension_select = capped_dimension_select(group_limit, dimensions, &mut params);
    let observation_table = observation_table(def.observation_source());
    let value_expr = grouped_value_expr(def);
    let value = transformed(def, "value".to_owned());
    let remainder_filter = if group_limit.include_remainder {
        ""
    } else {
        "WHERE group_rank > 0"
    };
    let limit = query_row_limit();
    let sql = format!(
        r"
        WITH scoped AS (
            SELECT
                *,
                {bucket} AS bucket_start
                {raw_dimensions}
            FROM {observation_table}
            WHERE {metric_where}
              {filter_where}
              AND {entity_predicate}
        ),
        ranked AS (
            SELECT
                *,
                {rank_expr} AS group_rank
            FROM scoped
        ),
        filtered AS (
            SELECT *
            FROM ranked
            {remainder_filter}
        ),
        aggregated AS (
            SELECT
                entity_id,
                bucket_start,
                group_rank,
                {value_expr} AS value,
                toUInt8(grouping(bucket_start)) AS is_total
            FROM filtered
            GROUP BY GROUPING SETS (
                (entity_id, bucket_start, group_rank),
                (entity_id, group_rank)
            )
        )
        SELECT
            entity_id,
            toString(bucket_start) AS bucket_start
            {dimension_select},
            {value} AS value,
            is_total,
            if(group_rank = 0, CAST(NULL AS Nullable(UInt32)), toNullable(group_rank)) AS rank,
            toUInt8(group_rank = 0) AS remainder,
            if(group_rank = 0, toNullable('Other'), CAST(NULL AS Nullable(String))) AS group_label
        FROM aggregated
        ORDER BY entity_id, group_rank, is_total, bucket_start
        LIMIT {limit}
        ",
        metric_where = metric_where(def, req.enforce_tenant_scope),
    );
    CompiledQuery { sql, params }
}

fn capped_rank_expr(
    group_limit: &ResolvedGroupLimit,
    dimension_count: usize,
    params: &mut Vec<String>,
) -> String {
    if group_limit.groups.is_empty() {
        return "toUInt32(0)".to_owned();
    }
    let mut branches = Vec::with_capacity(group_limit.groups.len() * 2 + 1);
    for group in &group_limit.groups {
        let comparisons = (0..dimension_count)
            .map(|index| format!("raw_dim_{index} = ?"))
            .collect::<Vec<_>>()
            .join(" AND ");
        params.extend(
            group
                .dimensions
                .iter()
                .map(|dimension| dimension.value.clone()),
        );
        branches.push(format!("({comparisons})"));
        branches.push(format!("toUInt32({})", group.rank));
    }
    branches.push("toUInt32(0)".to_owned());
    format!("multiIf({})", branches.join(", "))
}

fn capped_dimension_select(
    group_limit: &ResolvedGroupLimit,
    dimensions: &[String],
    params: &mut Vec<String>,
) -> String {
    let mut select = String::new();
    for (index, _) in dimensions.iter().enumerate() {
        let (value_alias, label_alias) = dimension_aliases(index);
        if group_limit.groups.is_empty() {
            let _ = write!(
                select,
                ", CAST(NULL AS Nullable(String)) AS {value_alias}, CAST(NULL AS Nullable(String)) AS {label_alias}"
            );
            continue;
        }
        let mut value_branches = Vec::with_capacity(group_limit.groups.len() * 2 + 1);
        let mut label_branches = Vec::with_capacity(group_limit.groups.len() * 2 + 1);
        let mut values = Vec::with_capacity(group_limit.groups.len());
        let mut labels = Vec::with_capacity(group_limit.groups.len());
        for group in &group_limit.groups {
            let dimension = &group.dimensions[index];
            value_branches.push(format!("group_rank = {}", group.rank));
            value_branches.push("toNullable(?)".to_owned());
            values.push(dimension.value.clone());
            label_branches.push(format!("group_rank = {}", group.rank));
            match &dimension.label {
                Some(label) => {
                    label_branches.push("toNullable(?)".to_owned());
                    labels.push(label.clone());
                }
                None => label_branches.push("CAST(NULL AS Nullable(String))".to_owned()),
            }
        }
        params.extend(values);
        params.extend(labels);
        value_branches.push("CAST(NULL AS Nullable(String))".to_owned());
        label_branches.push("CAST(NULL AS Nullable(String))".to_owned());
        let _ = write!(
            select,
            ", multiIf({}) AS {value_alias}, multiIf({}) AS {label_alias}",
            value_branches.join(", "),
            label_branches.join(", ")
        );
    }
    select
}

pub(crate) fn compile_breakdown_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    dimensions: &[String],
    filters: &[ValidatedDimensionFilter],
) -> CompiledQuery {
    let mut params = metric_params(def, req);
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_predicate = selected_entity_predicate(req, &mut params);
    let (dim_select, dim_group) = dimension_select_group(dimensions);
    let (source_select, source_group) = hidden_source_context(dimensions);
    let group = if dim_group.is_empty() {
        "entity_id".to_owned()
    } else {
        format!("entity_id, {dim_group}")
    };
    let group = format!("{group}{source_group}");
    let observation_table = observation_table(def.observation_source());
    let limit = query_row_limit();
    let value_expr = grouped_value_expr(def);
    let inner = format!(
        r"
        SELECT
            entity_id{dim_select}{source_select},
            {value_expr} AS value
        FROM {observation_table}
        WHERE {metric_where}
          {filter_where}
          AND {entity_predicate}
        GROUP BY {group}
        ORDER BY entity_id
        LIMIT {limit}
        ",
        metric_where = metric_where(def, req.enforce_tenant_scope),
    );
    let sql = transformed_single(def, inner);
    CompiledQuery { sql, params }
}

pub(crate) fn compile_rollup_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    dimensions: &[String],
    filters: &[ValidatedDimensionFilter],
    group_limit: Option<&ResolvedGroupLimit>,
) -> CompiledQuery {
    match group_limit {
        Some(group_limit) => {
            compile_capped_rollup_query(def, req, dimensions, filters, group_limit)
        }
        None => compile_uncapped_rollup_query(def, req, dimensions, filters),
    }
}

fn compile_uncapped_rollup_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    dimensions: &[String],
    filters: &[ValidatedDimensionFilter],
) -> CompiledQuery {
    let mut params = metric_params(def, req);
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_predicate = selected_entity_predicate(req, &mut params);
    let (dim_select, dim_group, dim_order) = ranking_dimension_select_group(dimensions);
    let observation_table = observation_table(def.observation_source());
    let value_expr = grouped_value_expr(def);
    let limit = query_row_limit();
    let inner = format!(
        r"
        SELECT
            {dim_select},
            {value_expr} AS value,
            uniqExact(entity_id) AS contributing_entity_count,
            CAST(NULL AS Nullable(UInt32)) AS rank,
            toUInt8(0) AS remainder,
            CAST(NULL AS Nullable(String)) AS group_label
        FROM {observation_table}
        WHERE {metric_where}
          {filter_where}
          AND {entity_predicate}
        GROUP BY {dim_group}
        ORDER BY {dim_order}
        LIMIT {limit}
        ",
        metric_where = metric_where(def, req.enforce_tenant_scope),
    );
    let sql = transformed_single(def, inner);
    CompiledQuery { sql, params }
}

fn compile_capped_rollup_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    dimensions: &[String],
    filters: &[ValidatedDimensionFilter],
    group_limit: &ResolvedGroupLimit,
) -> CompiledQuery {
    let mut params = metric_where_params(def, req);
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_predicate = selected_entity_predicate(req, &mut params);
    let raw_dimensions = dimensions.iter().enumerate().fold(
        String::new(),
        |mut raw_dimensions, (index, dimension)| {
            let _ = write!(
                raw_dimensions,
                ", {} AS raw_dim_{index}",
                dimension_value_expr(dimension)
            );
            raw_dimensions
        },
    );
    let rank_expr = capped_rank_expr(group_limit, dimensions.len(), &mut params);
    params.extend(grouped_value_params(def));
    let dimension_select = capped_dimension_select(group_limit, dimensions, &mut params);
    let observation_table = observation_table(def.observation_source());
    let value_expr = grouped_value_expr(def);
    let value = transformed(def, "value".to_owned());
    let remainder_filter = if group_limit.include_remainder {
        ""
    } else {
        "WHERE group_rank > 0"
    };
    let limit = query_row_limit();
    let sql = format!(
        r"
        WITH scoped AS (
            SELECT
                *
                {raw_dimensions}
            FROM {observation_table}
            WHERE {metric_where}
              {filter_where}
              AND {entity_predicate}
        ),
        ranked AS (
            SELECT
                *,
                {rank_expr} AS group_rank
            FROM scoped
        ),
        filtered AS (
            SELECT *
            FROM ranked
            {remainder_filter}
        ),
        aggregated AS (
            SELECT
                group_rank,
                {value_expr} AS value,
                uniqExact(entity_id) AS contributing_entity_count
            FROM filtered
            GROUP BY group_rank
        )
        SELECT
            group_rank{dimension_select},
            {value} AS value,
            contributing_entity_count,
            if(group_rank = 0, CAST(NULL AS Nullable(UInt32)), toNullable(group_rank)) AS rank,
            toUInt8(group_rank = 0) AS remainder,
            if(group_rank = 0, toNullable('Other'), CAST(NULL AS Nullable(String))) AS group_label
        FROM aggregated
        ORDER BY group_rank = 0, group_rank
        LIMIT {limit}
        ",
        metric_where = metric_where(def, req.enforce_tenant_scope),
    );
    CompiledQuery { sql, params }
}

// The per-group aggregate for single-metric queries (timeseries/breakdown),
// scoped by metric_where so no source/measure predicates repeat here. The
// ratio arm keeps its two SELECT placeholders, which metric_params leads
// with (SELECT binds before WHERE in text order).
fn grouped_value_expr(def: &MetricDefinition) -> String {
    match &def.spec {
        ComputationSpec::Sum { .. } => "sumIf(value, value IS NOT NULL)".to_owned(),
        ComputationSpec::Ratio {
            scale,
            denominator_aggregation,
            ..
        } => {
            let denominator = ratio_denominator_expr(
                *denominator_aggregation,
                "measure_key = ? AND value IS NOT NULL",
                "measure_key = ? AND subject_key IS NOT NULL",
            );
            format!(
                "{scale} * sumIfOrNull(value, measure_key = ? AND value IS NOT NULL) / nullIf({denominator}, 0)"
            )
        }
        ComputationSpec::Median { .. } => {
            "quantileExactIf(0.5)(value, value IS NOT NULL)".to_owned()
        }
        ComputationSpec::Percentile { q, .. } => {
            format!("quantileExactIf({q})(value, value IS NOT NULL)")
        }
        ComputationSpec::Stddev { .. } => "stddevSampIf(value, value IS NOT NULL)".to_owned(),
        ComputationSpec::DistinctCount { .. } => {
            "toFloat64(uniqExactIf(subject_key, subject_key IS NOT NULL))".to_owned()
        }
    }
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
    filters: &[ValidatedDimensionFilter],
) -> CompiledQuery {
    let mut params = metric_params(def, req);
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_predicate = selected_entity_predicate(req, &mut params);
    let observation_table = observation_table(def.observation_source());
    let bins = HISTOGRAM_BINS;
    let max_bin = HISTOGRAM_BINS - 1;
    let limit = query_row_limit();
    let sql = format!(
        r"
        WITH raw_events AS (
            SELECT
                entity_id,
                assumeNotNull({event_value}) AS event_value
            FROM {observation_table}
            WHERE {metric_where}
              {filter_where}
              AND {entity_predicate}
              AND value IS NOT NULL
        ),
        events AS (
            SELECT
                entity_id,
                event_value,
                min(event_value) OVER (PARTITION BY entity_id) AS entity_lo,
                max(event_value) OVER (PARTITION BY entity_id) AS entity_hi
            FROM raw_events
        )
        SELECT
            toString(assumeNotNull(events.entity_id)) AS entity_id,
            if(
                events.entity_hi = events.entity_lo,
                0,
                toUInt32(least({max_bin}, toInt64(floor(
                    (events.event_value - events.entity_lo) * {bins} / (events.entity_hi - events.entity_lo)
                ))))
            ) AS bin_idx,
            any(events.entity_lo) AS entity_lo,
            any(events.entity_hi) AS entity_hi,
            toUInt64(count()) AS bin_count
        FROM events
        GROUP BY entity_id, bin_idx
        ORDER BY entity_id, bin_idx
        LIMIT {limit}
        ",
        metric_where = metric_where(def, req.enforce_tenant_scope),
        event_value = transformed(def, "value".to_owned()),
    );
    CompiledQuery { sql, params }
}

// No collapsing here: cohort membership arrives as one row per
// (tenant, entity_id, cohort_key) — the cohort relation
// is canonical-grained and drops a person whose several HR emails claim
// different org units, so nothing here collapses or tie-breaks. Keep it that
// way: a pool that repairs its own input hides the input being wrong.
//
// WORKAROUND: ClickHouse inlines a WITH body once per reference, so a second
// reference to entity_values re-scans the whole observation window. Both
// shapes reference it exactly once and extract the target's own value from
// the peer rows with a conditional aggregate.
pub(crate) fn compile_peer_batch_query(
    defs: &[&MetricDefinition],
    req: &ValidatedMetricResultsRequest,
    cohort_key: &str,
    peer_population: PeerPopulation,
    filters: &[ValidatedDimensionFilter],
) -> CompiledQuery {
    match peer_population {
        PeerPopulation::DeclaredCohort => {
            compile_declared_cohort_peer_batch_query(defs, req, cohort_key, filters)
        }
        PeerPopulation::Tenant => compile_tenant_peer_batch_query(defs, req, filters),
    }
}

fn compile_declared_cohort_peer_batch_query(
    defs: &[&MetricDefinition],
    req: &ValidatedMetricResultsRequest,
    cohort_key: &str,
    filters: &[ValidatedDimensionFilter],
) -> CompiledQuery {
    let mut params = Vec::new();
    // The targets and cohort CTEs each scope by tenant/entity_type/cohort_key;
    // the targets CTE additionally filters person ids, bound between them.
    push_cohort_scope(&mut params, req, cohort_key);
    params.extend(req.entity.entity_ids());
    push_cohort_scope(&mut params, req, cohort_key);
    let value_selects = item_value_selects(defs, &mut params, period_alias);
    let metric_scope = shared_observation_where(defs, req, filters, &mut params);

    let entity_id_params = placeholders(req.entity.len());
    let observation_table = batch_observation_table(defs);
    let cohort_table = cohort_table(CohortSource::MetricEntityCohortsCurrent);
    let limit = query_row_limit();

    let carried = peer_carried_selects(defs);
    let stats_selects = per_target_aggregate_selects(defs);

    // Same switch as every other read (#1967): peer pools staying tenant-scoped
    // while the rest of the query bypasses would answer empty peers, which is
    // the failure the config flag exists to avoid.
    let tenant = tenant_predicate(req.enforce_tenant_scope);

    let sql = format!(
        r"
        WITH
        targets AS (
            SELECT entity_id, cohort_id
            FROM {cohort_table}
            WHERE {tenant} AND entity_type = ?
              AND cohort_key = ?
              AND entity_id IN ({entity_id_params})
        ),
        cohort AS (
            SELECT entity_id, cohort_id
            FROM {cohort_table}
            WHERE {tenant} AND entity_type = ?
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
        LEFT JOIN entity_values AS peer
            ON peer.cohort_id = targets.cohort_id
        GROUP BY targets.entity_id
        LIMIT {limit}
        SETTINGS join_use_nulls = 1
        "
    );
    CompiledQuery { sql, params }
}

fn compile_tenant_peer_batch_query(
    defs: &[&MetricDefinition],
    req: &ValidatedMetricResultsRequest,
    filters: &[ValidatedDimensionFilter],
) -> CompiledQuery {
    let mut params = req.entity.entity_ids();
    let value_selects = item_value_selects(defs, &mut params, period_alias);
    let metric_scope = shared_observation_where(defs, req, filters, &mut params);

    let entity_id_params = placeholders(req.entity.len());
    let observation_table = batch_observation_table(defs);
    let limit = query_row_limit();

    let carried = peer_carried_selects(defs);
    let captured_tuple = captured_target_tuple(defs);
    let population_stat_selects = population_stat_selects(defs);
    let result_selects = tenant_result_selects(defs);

    // INVARIANT: population stats aggregate once over the whole pool and the
    // requested targets' own values are captured in that same pass (bounded
    // by MAX_PERSON_IDS tuples), so cost stays one scan and one aggregation
    // regardless of how many targets the request carries.
    let sql = format!(
        r"
        WITH
        targets AS (
            SELECT arrayJoin([{entity_id_params}]) AS entity_id
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
                entity_id{carried}
            FROM metric_values
        ),
        population_stats AS (
            SELECT
                groupArrayIf({captured_tuple}, peer.entity_id IN (SELECT entity_id FROM targets)) AS target_rows{population_stat_selects}
            FROM entity_values AS peer
        )
        SELECT
            targets.entity_id AS entity_id{result_selects}
        FROM targets
        CROSS JOIN population_stats
        LIMIT {limit}
        SETTINGS join_use_nulls = 1
        "
    );
    CompiledQuery { sql, params }
}

// Per-item aggregate selects over the single `peer` reference, grouped by
// target: the target's own value plus the guarded stats block.
fn per_target_aggregate_selects(defs: &[&MetricDefinition]) -> String {
    let mut stats_selects = String::new();
    for item_index in 0..defs.len() {
        let value = period_alias(item_index);
        let aliases = peer_aliases(item_index);
        // INVARIANT: at most one peer row matches the target (the cohort
        // relation is canonical-grained; entity_values groups by entity_id),
        // so maxIf returns exactly that row's value — NULL when unobserved.
        let _ = write!(
            stats_selects,
            ",
            maxIf(peer.{value}, peer.entity_id = targets.entity_id) AS {target}",
            target = aliases.target,
        );
        push_peer_stat_selects(&mut stats_selects, item_index);
    }
    stats_selects
}

fn peer_carried_selects(defs: &[&MetricDefinition]) -> String {
    let mut carried = String::new();
    for (item_index, def) in defs.iter().enumerate() {
        let value = period_alias(item_index);
        let carried_value = transformed(def, format!("metric_values.{value}"));
        let _ = write!(
            carried,
            ",
                {carried_value} AS {value}"
        );
    }
    carried
}

fn captured_target_tuple(defs: &[&MetricDefinition]) -> String {
    let mut tuple = "tuple(peer.entity_id".to_owned();
    for item_index in 0..defs.len() {
        let value = period_alias(item_index);
        let _ = write!(tuple, ", peer.{value}");
    }
    tuple.push(')');
    tuple
}

fn population_stat_selects(defs: &[&MetricDefinition]) -> String {
    let mut selects = String::new();
    for item_index in 0..defs.len() {
        push_peer_stat_selects(&mut selects, item_index);
    }
    selects
}

fn tenant_result_selects(defs: &[&MetricDefinition]) -> String {
    let mut result_selects = String::new();
    for item_index in 0..defs.len() {
        let aliases = peer_aliases(item_index);
        // SAFETY: arrayFirst yields the default tuple when the target was not
        // captured; its Nullable elements read NULL, so an unmeasured target
        // stays NULL. Element 1 is the entity id, values start at 2.
        let _ = write!(
            result_selects,
            ",
            tupleElement(arrayFirst(t -> t.1 = targets.entity_id, population_stats.target_rows), {position}) AS {target},
            population_stats.{p25} AS {p25},
            population_stats.{median} AS {median},
            population_stats.{p75} AS {p75},
            population_stats.{min} AS {min},
            population_stats.{max} AS {max},
            population_stats.{n} AS {n}",
            position = item_index + 2,
            target = aliases.target,
            p25 = aliases.p25,
            median = aliases.median,
            p75 = aliases.p75,
            min = aliases.min,
            max = aliases.max,
            n = aliases.n,
        );
    }
    result_selects
}

fn push_peer_stat_selects(selects: &mut String, item_index: usize) {
    let value = period_alias(item_index);
    let aliases = peer_aliases(item_index);
    let observed = format!("peer.{value} IS NOT NULL");
    let pool = format!("uniqExactIf(peer.entity_id, {observed})");
    let quantiles = format!("quantilesExactIf(0.25, 0.5, 0.75)(peer.{value}, {observed})");
    let _ = write!(
        selects,
        ",
            if({pool} >= {min_peer_n}, toNullable({quantiles}[1]), NULL) AS {p25},
            if({pool} >= {min_peer_n}, toNullable({quantiles}[2]), NULL) AS {median},
            if({pool} >= {min_peer_n}, toNullable({quantiles}[3]), NULL) AS {p75},
            if({pool} >= {min_peer_n}, minIfOrNull(peer.{value}, {observed}), NULL) AS {min},
            if({pool} >= {min_peer_n}, maxIfOrNull(peer.{value}, {observed}), NULL) AS {max},
            toUInt64({pool}) AS {n}",
        p25 = aliases.p25,
        median = aliases.median,
        p75 = aliases.p75,
        min = aliases.min,
        max = aliases.max,
        n = aliases.n,
        min_peer_n = MIN_PEER_N,
    );
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
            denominator_aggregation,
        } => {
            // Ratio inputs share one source (enforced at definition load:
            // "ratio inputs must share one source"), so the numerator's
            // source_key scopes both halves. The numerator is OrNull: a tool
            // that reports the denominator but never the numerator measure
            // must read NULL (unknown split), not a fabricated 0. The
            // denominator needs no OrNull — both supported aggregations yield
            // 0 for no rows and nullIf already turns that into NULL.
            params.push(numerator.source_key.clone());
            params.push(numerator.measure_key.clone());
            params.push(numerator.source_key.clone());
            params.push(denominator.measure_key.clone());
            let denominator = ratio_denominator_expr(
                *denominator_aggregation,
                "source_key = ? AND measure_key = ? AND value IS NOT NULL",
                "source_key = ? AND measure_key = ? AND subject_key IS NOT NULL",
            );
            format!(
                "{scale} * sumIfOrNull(value, source_key = ? AND measure_key = ? AND value IS NOT NULL) / nullIf({denominator}, 0)"
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
        ComputationSpec::Percentile { value, q } => {
            // Honest-null like Median — same event-grain shape, different point
            // on the distribution.
            params.push(value.source_key.clone());
            params.push(value.measure_key.clone());
            format!(
                "quantileExactIfOrNull({q})(value, source_key = ? AND measure_key = ? AND value IS NOT NULL)"
            )
        }
        ComputationSpec::Stddev { value } => {
            // Honest-null like Median; sample stddev, so a single observation
            // reads NULL (no spread is measurable), never a fabricated 0.
            params.push(value.source_key.clone());
            params.push(value.measure_key.clone());
            "stddevSampIfOrNull(value, source_key = ? AND measure_key = ? AND value IS NOT NULL)"
                .to_owned()
        }
        ComputationSpec::DistinctCount { value } => {
            // OrNull like sum: an entity present via another measure but with
            // no rows for this one comes back NULL, not 0, so it never enters
            // a peer pool as a fabricated observation. The builder zero-fills
            // distinct counts (0 distinct subjects is a genuine zero) exactly
            // as it does sums. Counts distinct `subject_key`, not `value`.
            params.push(value.source_key.clone());
            params.push(value.measure_key.clone());
            // toFloat64 so the wide column is Float64, not a JSON-quoted
            // UInt64 (uniqExact's native type) the f64 row decoder rejects.
            // OrNull is preserved through the cast (NULL stays NULL).
            "toFloat64(uniqExactIfOrNull(subject_key, source_key = ? AND measure_key = ? AND subject_key IS NOT NULL))"
                .to_owned()
        }
    }
}

fn ratio_denominator_expr(
    aggregation: RatioDenominatorAggregation,
    sum_condition: &str,
    distinct_condition: &str,
) -> String {
    match aggregation {
        RatioDenominatorAggregation::Sum => format!("sumIf(value, {sum_condition})"),
        RatioDenominatorAggregation::DistinctCount => {
            format!("uniqExactIf(subject_key, {distinct_condition})")
        }
    }
}

// Applies the definition's post-aggregation transform to a computed value
// expression. Identity when the definition has none. Callers must pass an
// expression that is safe to repeat in SQL text (a column or alias
// reference, never one containing `?` placeholders) — the NULL-guarded
// clamp references it more than once.
fn transformed(def: &MetricDefinition, expr: String) -> String {
    match &def.transform {
        Some(transform) => transform.wrap_sql(&expr),
        None => expr,
    }
}

// Wraps a single-metric query in a transform projection stage. The raw
// aggregate stays in the inner query (its placeholders bind once); the
// transform references only the `value` alias.
fn transformed_single(def: &MetricDefinition, inner: String) -> String {
    if def.transform.is_none() {
        return inner;
    }
    let value = transformed(def, "value".to_owned());
    format!(
        r"
        SELECT
            * EXCEPT (value),
            {value} AS value
        FROM ({inner})
        "
    )
}

// Batch variant: re-projects each transformed item column by alias.
fn transformed_batch(
    defs: &[&MetricDefinition],
    inner: String,
    alias: fn(usize) -> String,
) -> String {
    if defs.iter().all(|def| def.transform.is_none()) {
        return inner;
    }
    let mut selects = String::new();
    for (item_index, def) in defs.iter().enumerate() {
        let value = alias(item_index);
        let expr = transformed(def, value.clone());
        let _ = write!(selects, ", {expr} AS {value}");
    }
    format!(
        r"
        SELECT
            entity_id{selects}
        FROM ({inner})
        "
    )
}

/// Push the `tenant_id`, `entity_type`, `cohort_key` values a cohort CTE's
/// `WHERE` binds, in that order. Called once per CTE (targets, cohort).
fn push_cohort_scope(
    params: &mut Vec<String>,
    req: &ValidatedMetricResultsRequest,
    cohort_key: &str,
) {
    params.push(req.tenant_id.to_string());
    params.push(req.entity.entity_type().to_owned());
    params.push(cohort_key.to_owned());
}

fn shared_observation_where(
    defs: &[&MetricDefinition],
    req: &ValidatedMetricResultsRequest,
    filters: &[ValidatedDimensionFilter],
    params: &mut Vec<String>,
) -> String {
    params.push(req.tenant_id.to_string());
    params.push(req.entity.entity_type().to_owned());
    params.push(req.from.to_string());
    params.push(req.to.to_string());
    let pairs = measure_pairs(defs);
    for (source_key, measure_key) in &pairs {
        params.push(source_key.clone());
        params.push(measure_key.clone());
    }
    let pair_placeholders = vec!["(?, ?)"; pairs.len()].join(", ");
    let tenant = tenant_predicate(req.enforce_tenant_scope);
    let mut where_clause = format!(
        "{tenant} AND entity_type = ? AND metric_date >= toDate(?) AND metric_date <= toDate(?) AND (source_key, measure_key) IN ({pair_placeholders})"
    );
    where_clause.push_str(&dimension_filter_where(filters, params));
    where_clause
}

fn dimension_filter_where(
    filters: &[ValidatedDimensionFilter],
    params: &mut Vec<String>,
) -> String {
    let mut sql = String::new();
    for filter in filters {
        let values = placeholders(filter.values.len());
        let _ = write!(
            sql,
            " AND indexOf(dimensions.1, '{dimension}') > 0 AND dimensions.2[indexOf(dimensions.1, '{dimension}')] IN ({values})",
            dimension = filter.dimension,
        );
        params.extend(filter.values.iter().cloned());
    }
    sql
}

fn measure_pairs(defs: &[&MetricDefinition]) -> BTreeSet<(String, String)> {
    defs.iter()
        .flat_map(|def| match &def.spec {
            ComputationSpec::Sum { value }
            | ComputationSpec::Median { value }
            | ComputationSpec::Percentile { value, .. }
            | ComputationSpec::Stddev { value }
            | ComputationSpec::DistinctCount { value } => {
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
    observation_table(def.observation_source())
}

// INVARIANT: every observation read leads with the tenant predicate, bound from
// the request's SecurityContext (never client SQL), so an enforced request
// scoped to tenant A cannot read tenant B's rows. `tenant_id` is the column the
// gold observation and cohort contract exposes; the value is the raw tenant
// UUID, the same representation the metric lineage stamps. The placeholder is
// first here and its value first in `metric_where_params` — keep the two in
// lockstep. When enforcement is off (the default until the ingest tenant is
// aligned, #1829) the term is a tautology that still binds the same one
// placeholder, so the param order is identical in both modes.
pub(crate) fn tenant_predicate(enforce: bool) -> &'static str {
    if enforce {
        "tenant_id = ?"
    } else {
        // Bypass: still consumes the bound tenant param (String = String, so no
        // type coercion), but `OR 1 = 1` makes it match every row — the
        // pre-#1967 behavior, without changing placeholder arity.
        "(tenant_id = ? OR 1 = 1)"
    }
}

fn metric_where(def: &MetricDefinition, enforce_tenant_scope: bool) -> String {
    let tenant = tenant_predicate(enforce_tenant_scope);
    match &def.spec {
        ComputationSpec::Sum { .. }
        | ComputationSpec::Median { .. }
        | ComputationSpec::Percentile { .. }
        | ComputationSpec::Stddev { .. }
        | ComputationSpec::DistinctCount { .. } => {
            format!(
                "{tenant} AND source_key = ? AND entity_type = ? AND metric_date >= toDate(?) AND metric_date <= toDate(?) AND measure_key = ?"
            )
        }
        ComputationSpec::Ratio { .. } => {
            format!(
                "{tenant} AND source_key = ? AND entity_type = ? AND metric_date >= toDate(?) AND metric_date <= toDate(?) AND measure_key IN (?, ?)"
            )
        }
    }
}

fn metric_params(def: &MetricDefinition, req: &ValidatedMetricResultsRequest) -> Vec<String> {
    let mut params = grouped_value_params(def);
    params.extend(metric_where_params(def, req));
    params
}

fn grouped_value_params(def: &MetricDefinition) -> Vec<String> {
    match &def.spec {
        ComputationSpec::Ratio {
            numerator,
            denominator,
            ..
        } => vec![
            numerator.measure_key.clone(),
            denominator.measure_key.clone(),
        ],
        ComputationSpec::Sum { .. }
        | ComputationSpec::Median { .. }
        | ComputationSpec::Percentile { .. }
        | ComputationSpec::Stddev { .. }
        | ComputationSpec::DistinctCount { .. } => Vec::new(),
    }
}

fn metric_where_params(def: &MetricDefinition, req: &ValidatedMetricResultsRequest) -> Vec<String> {
    match &def.spec {
        ComputationSpec::Sum { value }
        | ComputationSpec::Median { value }
        | ComputationSpec::Percentile { value, .. }
        | ComputationSpec::Stddev { value }
        | ComputationSpec::DistinctCount { value } => vec![
            req.tenant_id.to_string(),
            value.source_key.clone(),
            req.entity.entity_type().to_owned(),
            req.from.to_string(),
            req.to.to_string(),
            value.measure_key.clone(),
        ],
        ComputationSpec::Ratio {
            numerator,
            denominator,
            ..
        } => vec![
            req.tenant_id.to_string(),
            numerator.source_key.clone(),
            req.entity.entity_type().to_owned(),
            req.from.to_string(),
            req.to.to_string(),
            numerator.measure_key.clone(),
            denominator.measure_key.clone(),
        ],
    }
}

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}

fn selected_entity_predicate(
    req: &ValidatedMetricResultsRequest,
    params: &mut Vec<String>,
) -> String {
    match &req.entity {
        ValidatedEntitySelection::Person { ids } => {
            params.extend(ids.iter().map(Uuid::to_string));
            format!("entity_id IN ({})", placeholders(ids.len()))
        }
        ValidatedEntitySelection::Tenant { .. } => "entity_id = tenant_id".to_owned(),
    }
}

// INVARIANT: the bucket key must be non-nullable — a custom source may
// declare `metric_date` as `Nullable(Date)`, and GROUPING SETS fills the
// totals row's absent key with NULL instead of the default date, which the
// row parser rejects. The date-range predicate has already dropped NULL
// dates, so `assumeNotNull` never sees one.
fn bucket_expr(bucket: Bucket) -> &'static str {
    match bucket {
        Bucket::Day => "assumeNotNull(metric_date)",
        Bucket::Week => "toStartOfWeek(assumeNotNull(metric_date), 1)",
        Bucket::Month => "toStartOfMonth(assumeNotNull(metric_date))",
    }
}

fn observation_table(source: &ObservationSource) -> String {
    source.render_from_clause()
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

fn hidden_source_context(dimensions: &[String]) -> (String, String) {
    if !dimensions.iter().any(|dimension| dimension == "repository") {
        return (String::new(), String::new());
    }
    let provider = dimension_value_expr("source");
    let source_id = dimension_value_expr("source_id");
    (
        format!(", {provider} AS link_source_provider, {source_id} AS link_source_id"),
        ", link_source_provider, link_source_id".to_owned(),
    )
}

fn ranking_dimension_select_group(dimensions: &[String]) -> (String, String, String) {
    let mut select = Vec::with_capacity(dimensions.len() * 2);
    let mut group = Vec::with_capacity(dimensions.len());
    let mut order = Vec::with_capacity(dimensions.len());
    for (index, dimension) in dimensions.iter().enumerate() {
        let (value_alias, label_alias) = dimension_aliases(index);
        let value = dimension_value_expr(dimension);
        let label = dimension_label_expr(dimension);
        select.push(format!("{value} AS {value_alias}"));
        select.push(format!(
            "argMax({label}, tuple(metric_date, {label})) AS {label_alias}"
        ));
        group.push(value_alias.clone());
        order.push(value_alias);
    }
    (select.join(", "), group.join(", "), order.join(", "))
}

// `indexOf(dimensions.1, key)` locates the matching tuple by its key column in
// one pass (0 when absent), then positional access into the value (`.2`) and
// label (`.3`) columns reuses that index — replacing three `arrayFilter`
// materializations of the tuple array per row with cheap column scans.
fn dimension_value_expr(dimension: &str) -> String {
    format!(
        r"
        if(
            indexOf(dimensions.1, '{dimension}') = 0,
            '{UNKNOWN_DIMENSION_VALUE}',
            coalesce(dimensions.2[indexOf(dimensions.1, '{dimension}')], '{UNKNOWN_DIMENSION_VALUE}')
        )
        "
    )
}

fn dimension_label_expr(dimension: &str) -> String {
    format!(
        r"
        if(
            indexOf(dimensions.1, '{dimension}') = 0,
            '{UNKNOWN_DIMENSION_LABEL}',
            coalesce(dimensions.3[indexOf(dimensions.1, '{dimension}')], '{UNKNOWN_DIMENSION_LABEL}')
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
    use uuid::Uuid;

    use crate::domain::metric_definitions::definition::ValueTransform;
    use crate::domain::metric_results::batch::{RankedDimension, RankedGroup};
    use crate::domain::metric_results::validation::ValidatedEntitySelection;
    use chrono::NaiveDate;

    use crate::domain::metric_definitions::definition::{
        CustomObservationSql, MetricBase, MetricDirection, MetricFormat, MetricInput,
        MetricInputRole, ObservationRelation, ObservationSource,
    };

    fn base(dimensions: Vec<&str>) -> MetricBase {
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
            peer_cohort_key: Some("org_unit".to_owned()),
            allowed_dimensions: dimensions.into_iter().map(str::to_owned).collect(),
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
        }
    }

    fn median_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(vec!["source"]),
            spec: ComputationSpec::Median {
                value: input(MetricInputRole::Value, "pr_cycle_hours"),
            },
        }
    }

    fn distinct_count_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(vec!["tool"]),
            spec: ComputationSpec::DistinctCount {
                value: input(MetricInputRole::Value, "active_day"),
            },
        }
    }

    fn sum_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(vec!["tool"]),
            spec: ComputationSpec::Sum {
                value: input(MetricInputRole::Value, "accepted_lines"),
            },
        }
    }

    fn ratio_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(vec!["tool"]),
            spec: ComputationSpec::Ratio {
                numerator: input(MetricInputRole::Numerator, "accepted_edit_actions"),
                denominator: input(MetricInputRole::Denominator, "tool_use_offered"),
                scale: 100.0,
                denominator_aggregation: RatioDenominatorAggregation::Sum,
            },
        }
    }

    #[test]
    fn ratio_can_count_distinct_denominator_subjects() {
        let mut metric = ratio_metric();
        let ComputationSpec::Ratio {
            denominator_aggregation,
            ..
        } = &mut metric.spec
        else {
            panic!("fixture must be ratio");
        };
        *denominator_aggregation = RatioDenominatorAggregation::DistinctCount;

        let query = compile_timeseries_query(&metric, &request(), Bucket::Week, &[], &[], None);

        assert!(
            query
                .sql
                .contains("uniqExactIf(subject_key, measure_key = ? AND subject_key IS NOT NULL)")
        );
        assert_eq!(query.params[0], "accepted_edit_actions");
        assert_eq!(query.params[1], "tool_use_offered");
    }

    const TEST_TENANT: uuid::Uuid = uuid::Uuid::from_u128(0x1967);
    const TEST_TENANT_STR: &str = "00000000-0000-0000-0000-000000001967";

    fn request() -> ValidatedMetricResultsRequest {
        ValidatedMetricResultsRequest {
            tenant_id: TEST_TENANT,
            entity: ValidatedEntitySelection::Person {
                ids: vec![Uuid::from_u128(0xa), Uuid::from_u128(0xb)],
            },
            from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap_or_default(),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap_or_default(),
            metrics: Vec::new(),
            enforce_tenant_scope: true,
        }
    }

    #[test]
    fn period_batch_binds_item_params_then_scope_then_pairs_then_entities() {
        let (sum, ratio) = (sum_metric(), ratio_metric());
        let query = compile_period_batch_query(&[&sum, &ratio], &request(), &[]);
        assert!(query.sql.contains("FROM insight.ai_metric_observations"));
        assert!(
            query
                .sql
                .contains("WHERE tenant_id = ? AND entity_type = ?")
        );
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
                // shared scope (tenant predicate leads)
                TEST_TENANT_STR,
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
                // person ids
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
            ]
        );
    }

    #[test]
    fn tenant_queries_match_the_entity_to_its_storage_partition() {
        let sum = sum_metric();
        let mut req = request();
        req.entity = ValidatedEntitySelection::Tenant { id: TEST_TENANT };

        let query = compile_period_batch_query(&[&sum], &req, &[]);

        assert!(query.sql.contains("AND entity_id = tenant_id"));
        assert!(!query.params.iter().any(|value| value == "default"));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    const CUSTOM_SQL: &str = "SELECT tenant_id, source_key, entity_type, entity_id, \
        metric_date, measure_key, observed_at, value, subject_key, dimensions FROM joined";

    fn custom_sum_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(vec!["tool"]),
            spec: ComputationSpec::Sum {
                value: MetricInput {
                    role: MetricInputRole::Value,
                    observation: ObservationSource::Custom(CustomObservationSql::new(
                        CUSTOM_SQL.to_owned(),
                    )),
                    source_key: "custom_ai_usage".to_owned(),
                    measure_key: "accepted_lines".to_owned(),
                },
            },
        }
    }

    #[test]
    fn custom_source_reads_from_the_inline_sql_with_the_tenant_filter_outside() {
        // A custom source's observation SQL becomes the `FROM` target as a
        // parenthesized subquery; the tenant predicate (and every other scope
        // term) is applied by the wrap OUTSIDE it, exactly as for a managed
        // relation, so a custom metric inherits identical tenant scoping.
        let def = custom_sum_metric();
        let ts = compile_timeseries_query(&def, &request(), Bucket::Day, &[], &[], None);
        assert!(ts.sql.contains(&format!("FROM ({CUSTOM_SQL})")));
        assert!(!ts.sql.contains("insight.ai_metric_observations"));
        assert!(ts.sql.contains("WHERE tenant_id = ?"));
        assert_eq!(ts.params.first().map(String::as_str), Some(TEST_TENANT_STR));
        assert_eq!(ts.sql.matches('?').count(), ts.params.len());

        // The batch path routes through the same from_clause.
        let batch = compile_period_batch_query(&[&def], &request(), &[]);
        assert!(batch.sql.contains(&format!("FROM ({CUSTOM_SQL})")));
    }

    #[test]
    fn period_batch_of_one_uses_wide_aliases() {
        let sum = sum_metric();
        let query = compile_period_batch_query(&[&sum], &request(), &[]);
        assert!(query.sql.contains("AS m0"));
        assert!(!query.sql.contains("AS value"));
    }

    #[test]
    fn ratio_item_binds_numerator_source_for_both_halves() {
        let ratio = ratio_metric();
        let query = compile_period_batch_query(&[&ratio], &request(), &[]);
        // Ratio inputs share one source by the definition-load invariant;
        // both sumIf halves and the pruning pair carry the numerator's key.
        assert_eq!(
            query.params,
            vec![
                "ai_usage",
                "accepted_edit_actions",
                "ai_usage",
                "tool_use_offered",
                TEST_TENANT_STR,
                "person",
                "2026-01-01",
                "2026-01-31",
                "ai_usage",
                "accepted_edit_actions",
                "ai_usage",
                "tool_use_offered",
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
            ]
        );
    }

    #[test]
    fn tenant_predicate_leads_and_binds_context_tenant_on_every_contract_read() {
        let sum = sum_metric();

        let ts = compile_timeseries_query(&sum, &request(), Bucket::Day, &[], &[], None);
        assert!(ts.sql.contains("WHERE tenant_id = ?"), "timeseries read");
        assert_eq!(ts.params.first().map(String::as_str), Some(TEST_TENANT_STR));
        assert_eq!(ts.sql.matches('?').count(), ts.params.len());

        let rank = compile_group_ranking_query(&sum, &request(), &["tool".to_owned()], &[], 5);
        assert!(rank.sql.contains("WHERE tenant_id = ?"), "ranking read");
        assert_eq!(
            rank.params.first().map(String::as_str),
            Some(TEST_TENANT_STR)
        );

        // The peer query reads the contract three times (targets, cohort,
        // metric_values); each must carry the tenant predicate and its value.
        let peer = compile_peer_batch_query(
            &[&sum],
            &request(),
            "org_unit",
            PeerPopulation::DeclaredCohort,
            &[],
        );
        assert_eq!(peer.sql.matches("tenant_id = ?").count(), 3);
        assert_eq!(
            peer.params
                .iter()
                .filter(|p| p.as_str() == TEST_TENANT_STR)
                .count(),
            3
        );
        assert_eq!(peer.sql.matches('?').count(), peer.params.len());
    }

    #[test]
    fn tenant_scope_disabled_bypasses_the_filter_but_keeps_param_arity() {
        let sum = sum_metric();
        let mut req = request();
        req.enforce_tenant_scope = false;

        // With enforcement off, every contract read swaps the exact-match term
        // for the tautology, so nothing filters by tenant — yet the placeholder
        // (and its bound value) stays in place, so param arity is unchanged.
        let ts = compile_timeseries_query(&sum, &req, Bucket::Day, &[], &[], None);
        assert!(
            ts.sql.contains("(tenant_id = ? OR 1 = 1)"),
            "timeseries uses the bypass term"
        );
        assert!(
            !ts.sql.contains("WHERE tenant_id = ?"),
            "no exact-match tenant term when bypassed"
        );
        assert_eq!(ts.sql.matches('?').count(), ts.params.len());
        assert_eq!(ts.params.first().map(String::as_str), Some(TEST_TENANT_STR));

        // All three peer reads (targets, cohort, metric_values) bypass together.
        let peer = compile_peer_batch_query(
            &[&sum],
            &req,
            "org_unit",
            PeerPopulation::DeclaredCohort,
            &[],
        );
        assert_eq!(peer.sql.matches("(tenant_id = ? OR 1 = 1)").count(), 3);
        assert_eq!(peer.sql.matches('?').count(), peer.params.len());
    }

    #[test]
    fn timeseries_query_buckets_on_a_non_nullable_date() {
        for (bucket, expr) in [
            (Bucket::Day, "assumeNotNull(metric_date)"),
            (Bucket::Week, "toStartOfWeek(assumeNotNull(metric_date), 1)"),
            (Bucket::Month, "toStartOfMonth(assumeNotNull(metric_date))"),
        ] {
            let query = compile_timeseries_query(&sum_metric(), &request(), bucket, &[], &[], None);
            assert!(
                query
                    .sql
                    .contains(&format!("toString({expr}) AS bucket_start"))
            );
            assert!(query.sql.contains("GROUP BY GROUPING SETS"));
        }
    }

    fn resolved_limit(include_remainder: bool) -> ResolvedGroupLimit {
        ResolvedGroupLimit {
            groups: vec![
                RankedGroup {
                    rank: 1,
                    dimensions: vec![RankedDimension {
                        value: "cursor".to_owned(),
                        label: Some("Cursor".to_owned()),
                    }],
                },
                RankedGroup {
                    rank: 2,
                    dimensions: vec![RankedDimension {
                        value: UNKNOWN_DIMENSION_VALUE.to_owned(),
                        label: Some(UNKNOWN_DIMENSION_LABEL.to_owned()),
                    }],
                },
            ],
            include_remainder,
        }
    }

    #[test]
    fn ranking_query_is_global_transformed_and_deterministic() {
        let mut def = sum_metric();
        def.transform = Some(ValueTransform {
            multiplier: Some(2.0),
            ..ValueTransform::default()
        });
        let query = compile_group_ranking_query(&def, &request(), &["tool".to_owned()], &[], 10);
        assert!(!query.sql.contains("GROUP BY entity_id"));
        assert!(query.sql.contains("2.0 * (value) AS value"));
        assert!(query.sql.contains("WHERE value IS NOT NULL"));
        assert!(query.sql.contains("ORDER BY value DESC, dim_0_value"));
        assert!(query.sql.contains("LIMIT 10"));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn capped_timeseries_freezes_groups_and_aggregates_the_remainder() {
        let dimensions = vec!["tool".to_owned()];
        for def in [
            sum_metric(),
            ratio_metric(),
            median_metric(),
            distinct_count_metric(),
        ] {
            let query = compile_timeseries_query(
                &def,
                &request(),
                Bucket::Week,
                &dimensions,
                &[],
                Some(&resolved_limit(true)),
            );
            assert!(query.sql.contains("AS group_rank"));
            assert!(query.sql.contains("GROUP BY GROUPING SETS"));
            assert!(query.sql.contains("(entity_id, group_rank)"));
            assert!(query.sql.contains("toNullable('Other')"));
            assert!(query.sql.contains("group_rank = 0"));
            assert!(!query.sql.contains("WHERE group_rank > 0"));
            assert_eq!(query.sql.matches('?').count(), query.params.len());
            assert!(query.params.windows(4).any(|values| {
                values
                    == [
                        "cursor",
                        UNKNOWN_DIMENSION_VALUE,
                        "Cursor",
                        UNKNOWN_DIMENSION_LABEL,
                    ]
            }));
        }
    }

    #[test]
    fn capped_timeseries_uses_one_aggregation_pipeline_for_points_and_totals() {
        let query = compile_timeseries_query(
            &ratio_metric(),
            &request(),
            Bucket::Day,
            &["tool".to_owned()],
            &[],
            Some(&resolved_limit(false)),
        );
        assert_eq!(query.sql.matches("sumIfOrNull(value").count(), 1);
        assert_eq!(query.sql.matches("nullIf(sumIf(value").count(), 1);
        assert_eq!(query.sql.matches("aggregated AS").count(), 1);
        assert!(query.sql.contains("WHERE group_rank > 0"));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn rollup_aggregates_values_and_entity_counts_without_entity_grain() {
        for def in [sum_metric(), ratio_metric(), median_metric()] {
            let query = compile_rollup_query(&def, &request(), &["tool".to_owned()], &[], None);

            assert!(
                query
                    .sql
                    .contains("uniqExact(entity_id) AS contributing_entity_count")
            );
            assert!(query.sql.contains("GROUP BY dim_0_value"));
            assert!(!query.sql.contains("GROUP BY entity_id"));
            assert_eq!(query.sql.matches('?').count(), query.params.len());
        }
    }

    #[test]
    fn capped_rollup_recomputes_values_and_entity_counts_for_remainder() {
        let query = compile_rollup_query(
            &ratio_metric(),
            &request(),
            &["tool".to_owned()],
            &[],
            Some(&resolved_limit(true)),
        );

        assert!(query.sql.contains("AS group_rank"));
        assert!(query.sql.contains("GROUP BY group_rank"));
        assert!(
            query
                .sql
                .contains("uniqExact(entity_id) AS contributing_entity_count")
        );
        assert!(query.sql.contains("toNullable('Other')"));
        assert!(query.sql.contains("ORDER BY group_rank = 0, group_rank"));
        assert_eq!(query.sql.matches("sumIfOrNull(value").count(), 1);
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn empty_ranking_routes_displayed_data_to_the_remainder() {
        let query = compile_timeseries_query(
            &sum_metric(),
            &request(),
            Bucket::Day,
            &["tool".to_owned()],
            &[],
            Some(&ResolvedGroupLimit {
                groups: vec![],
                include_remainder: true,
            }),
        );
        assert!(query.sql.contains("toUInt32(0) AS group_rank"));
        assert!(query.sql.contains("toNullable('Other')"));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn dimensioned_query_emits_value_and_label_aliases() {
        let query = compile_breakdown_query(&sum_metric(), &request(), &["tool".to_owned()], &[]);
        assert!(query.sql.contains("AS dim_0_value"));
        assert!(query.sql.contains("AS dim_0_label"));
        assert!(query.sql.contains("indexOf(dimensions.1, 'tool')"));
        assert!(
            query
                .sql
                .contains("GROUP BY entity_id, dim_0_value, dim_0_label")
        );
    }

    #[test]
    fn repository_breakdown_namespaces_hidden_source_context() {
        let query =
            compile_breakdown_query(&sum_metric(), &request(), &["repository".to_owned()], &[]);

        assert!(query.sql.contains("AS link_source_provider"));
        assert!(query.sql.contains("AS link_source_id"));
        assert!(query.sql.contains("link_source_provider, link_source_id"));
        assert!(!query.sql.contains(" AS source_id"));
    }

    #[test]
    fn peer_batch_keeps_cohort_ctes_and_param_order() {
        let sum = sum_metric();
        let query = compile_peer_batch_query(
            &[&sum],
            &request(),
            "org_unit",
            PeerPopulation::DeclaredCohort,
            &[],
        );
        assert!(
            query
                .sql
                .contains("FROM insight.metric_entity_cohorts_current")
        );
        assert_eq!(
            query.params,
            vec![
                // targets CTE (tenant predicate leads every read)
                TEST_TENANT_STR,
                "person",
                "org_unit",
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                // cohort CTE
                TEST_TENANT_STR,
                "person",
                "org_unit",
                // item value selects
                "ai_usage",
                "accepted_lines",
                // metric_values shared scope
                TEST_TENANT_STR,
                "person",
                "2026-01-01",
                "2026-01-31",
                "ai_usage",
                "accepted_lines",
            ]
        );
    }

    #[test]
    fn flat_peer_batch_uses_every_measured_person_as_one_population() {
        let sum = sum_metric();
        let query =
            compile_peer_batch_query(&[&sum], &request(), "org_unit", PeerPopulation::Tenant, &[]);

        assert!(!query.sql.contains("metric_entity_cohorts_current"));
        assert!(query.sql.contains("arrayJoin([?, ?]) AS entity_id"));
        assert!(query.sql.contains("CROSS JOIN population_stats"));
        assert!(!query.sql.contains("ON 1 = 1"));
        // Stats aggregate once over the pool; the targets' own values ride the
        // same pass as captured tuples, so target count never multiplies the
        // aggregation state.
        assert!(query.sql.contains(
            "groupArrayIf(tuple(peer.entity_id, peer.m0), \
             peer.entity_id IN (SELECT entity_id FROM targets)) AS target_rows"
        ));
        assert!(query.sql.contains(
            "tupleElement(arrayFirst(t -> t.1 = targets.entity_id, \
             population_stats.target_rows), 2) AS m0_target"
        ));
        assert!(
            query
                .sql
                .contains("population_stats.m0_median AS m0_median")
        );
        assert!(
            query
                .sql
                .contains("toUInt64(uniqExactIf(peer.entity_id, peer.m0 IS NOT NULL)) AS m0_n"),
            "an empty observation window still yields the one aggregate row, so n reads 0"
        );
        assert_eq!(query.sql.matches("tenant_id = ?").count(), 1);
        assert_eq!(
            query.params,
            vec![
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                "ai_usage",
                "accepted_lines",
                TEST_TENANT_STR,
                "person",
                "2026-01-01",
                "2026-01-31",
                "ai_usage",
                "accepted_lines",
            ]
        );
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn peer_batches_read_the_observation_window_once() {
        // ClickHouse expands a WITH body once per reference, so a second
        // reference to entity_values would scan the whole observation window
        // again. Each shape must reference it exactly once (definition + one
        // use) and extract the target's own value from that single reference.
        let sum = sum_metric();
        for population in [PeerPopulation::DeclaredCohort, PeerPopulation::Tenant] {
            let query = compile_peer_batch_query(&[&sum], &request(), "org_unit", population, &[]);
            assert_eq!(
                query.sql.matches("entity_values").count(),
                2,
                "one definition and one reference, got:\n{}",
                query.sql
            );
            assert!(
                !query.sql.contains("AS target_values"),
                "the target join re-scans the window; extract from the single peer reference"
            );
        }
    }

    #[test]
    fn peer_batch_never_fabricates_zero_observations() {
        // Honest-null through the runtime: cohort members without observed
        // values stay NULL and drop out of the pool per metric — absence of
        // rows cannot be distinguished from "not covered by the source", so
        // the peer query must not invent zeros for them.
        let (sum, ratio) = (sum_metric(), ratio_metric());
        let query = compile_peer_batch_query(
            &[&sum, &ratio],
            &request(),
            "org_unit",
            PeerPopulation::DeclaredCohort,
            &[],
        );
        assert!(query.sql.contains("sumIfOrNull"));
        assert!(!query.sql.contains("coalesce"));
        assert!(query.sql.contains("metric_values.m0 AS m0"));
    }

    #[test]
    fn peer_batch_guards_every_percentile_per_item() {
        let (sum, ratio) = (sum_metric(), ratio_metric());
        let query = compile_peer_batch_query(
            &[&sum, &ratio],
            &request(),
            "org_unit",
            PeerPopulation::DeclaredCohort,
            &[],
        );
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
        // Quartiles come from one `quantilesExactIf` per item (single sort),
        // not three separate `quantileExactIf` calls.
        for item in 0..2 {
            assert!(query.sql.contains(&format!(
                "quantilesExactIf(0.25, 0.5, 0.75)(peer.m{item}, peer.m{item} IS NOT NULL)"
            )));
        }
        assert!(!query.sql.contains("quantileExactIf(0.25)"));
        // The cohort relation is canonical-grained (one row per person and
        // cohort_key, contested membership already dropped), so the pool reads
        // it straight. A collapse here would repair a broken input silently.
        assert!(
            !query.sql.contains("HAVING"),
            "no collapsing: the cohort relation already arrives at person grain"
        );
        assert!(
            query
                .sql
                .contains("AND cohort_id IN (SELECT cohort_id FROM targets)"),
            "the pool is exactly the target cohorts' membership"
        );
        // Honest-null must not depend on server config or column typing.
        assert!(query.sql.contains("SETTINGS join_use_nulls = 1"));
        assert!(query.sql.contains("GROUP BY targets.entity_id"));
    }

    #[test]
    fn queries_carry_row_limit() {
        let (sum, ratio) = (sum_metric(), ratio_metric());
        let limit = format!("LIMIT {}", query_row_limit());
        assert!(
            compile_period_batch_query(&[&sum], &request(), &[])
                .sql
                .contains(&limit)
        );
        assert!(
            compile_peer_batch_query(
                &[&ratio],
                &request(),
                "org_unit",
                PeerPopulation::DeclaredCohort,
                &[],
            )
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
        let (sum, median, ratio, distinct) = (
            sum_metric(),
            median_metric(),
            ratio_metric(),
            distinct_count_metric(),
        );
        for query in [
            compile_period_batch_query(&[&sum, &median, &ratio, &distinct], &request(), &[]),
            compile_peer_batch_query(
                &[&sum, &median, &ratio, &distinct],
                &request(),
                "org_unit",
                PeerPopulation::DeclaredCohort,
                &[],
            ),
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
            compile_period_batch_query(&[&median_metric()], &request(), &[]),
            compile_peer_batch_query(
                &[&median_metric()],
                &request(),
                "org_unit",
                PeerPopulation::DeclaredCohort,
                &[],
            ),
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
        let ts =
            compile_timeseries_query(&median_metric(), &request(), Bucket::Week, &[], &[], None);
        assert!(
            ts.sql
                .contains("quantileExactIf(0.5)(value, value IS NOT NULL)")
        );
        assert!(ts.sql.contains("GROUP BY GROUPING SETS"));
        let bd = compile_breakdown_query(&median_metric(), &request(), &["source".to_owned()], &[]);
        assert!(
            bd.sql
                .contains("quantileExactIf(0.5)(value, value IS NOT NULL)")
        );
    }

    #[test]
    fn distinct_count_batches_as_a_uniq_ornull_column() {
        // A distinct-count metric joins the period/peer batch as one wide
        // column counting distinct subject_key. OrNull so an entity present
        // via another measure but with no rows here comes back NULL, not 0 —
        // the builder zero-fills distinct counts like sums. Lockstep holds.
        for query in [
            compile_period_batch_query(&[&distinct_count_metric()], &request(), &[]),
            compile_peer_batch_query(
                &[&distinct_count_metric()],
                &request(),
                "org_unit",
                PeerPopulation::DeclaredCohort,
                &[],
            ),
        ] {
            assert!(
                query
                    .sql
                    .contains("uniqExactIfOrNull(subject_key, source_key = ? AND measure_key = ?"),
                "distinct count must batch as an OrNull uniqExact column over subject_key"
            );
            assert_eq!(query.sql.matches('?').count(), query.params.len());
        }
    }

    #[test]
    fn ratio_numerator_never_fabricates_zero() {
        // A tool that reports the denominator measure but never the numerator
        // (e.g. a chat source with totals but no DM split) must read NULL,
        // not 0%: the numerator aggregates OrNull in every query shape, while
        // the denominator relies on nullIf alone.
        let batched = compile_period_batch_query(&[&ratio_metric()], &request(), &[]);
        let ts =
            compile_timeseries_query(&ratio_metric(), &request(), Bucket::Week, &[], &[], None);
        let bd = compile_breakdown_query(&ratio_metric(), &request(), &["tool".to_owned()], &[]);
        assert!(
            batched
                .sql
                .contains("100 * sumIfOrNull(value, source_key = ?")
        );
        for query in [&ts, &bd] {
            assert!(
                query
                    .sql
                    .contains("100 * sumIfOrNull(value, measure_key = ?")
            );
            assert!(query.sql.contains("nullIf(sumIf(value, measure_key = ?"));
        }
    }

    #[test]
    fn distinct_count_single_views_count_distinct_subject_key() {
        let ts = compile_timeseries_query(
            &distinct_count_metric(),
            &request(),
            Bucket::Week,
            &[],
            &[],
            None,
        );
        assert!(
            ts.sql
                .contains("uniqExactIf(subject_key, subject_key IS NOT NULL)")
        );
        assert!(ts.sql.contains("GROUP BY GROUPING SETS"));
        let bd = compile_breakdown_query(
            &distinct_count_metric(),
            &request(),
            &["tool".to_owned()],
            &[],
        );
        assert!(
            bd.sql
                .contains("uniqExactIf(subject_key, subject_key IS NOT NULL)")
        );
    }

    #[test]
    fn histogram_query_bins_deterministically_from_entity_bounds() {
        let query = compile_histogram_query(&median_metric(), &request(), &[]);
        assert!(
            query
                .sql
                .contains("min(event_value) OVER (PARTITION BY entity_id) AS entity_lo")
        );
        assert!(
            query
                .sql
                .contains("max(event_value) OVER (PARTITION BY entity_id) AS entity_hi")
        );
        assert!(query.sql.contains("least(9,"));
        assert!(query.sql.contains("* 10 /"));
        assert!(query.sql.contains("GROUP BY entity_id, bin_idx"));
        assert!(query.sql.contains("events.entity_hi = events.entity_lo"));
        // Bounds come from a window pass, not a self-join back to the events.
        assert!(!query.sql.contains("JOIN"));
        // Deterministic arithmetic only — never the adaptive aggregate.
        assert!(!query.sql.contains("histogram("));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn transform_wraps_every_query_shape() {
        let mut def = ratio_metric();
        def.transform = Some(ValueTransform {
            clamp_max: Some(100.0),
            ..ValueTransform::default()
        });
        let ts = compile_timeseries_query(&def, &request(), Bucket::Day, &[], &[], None);
        let bd = compile_breakdown_query(&def, &request(), &[], &[]);
        for query in [&ts, &bd] {
            assert!(
                query
                    .sql
                    .contains("if((value) IS NULL, NULL, least(100.0, value)) AS value"),
                "transform must re-project the value alias: {}",
                query.sql
            );
            assert_eq!(query.sql.matches('?').count(), query.params.len());
        }
        // Histogram (median-only, hence its own def) must bin the transformed
        // event value under its own alias, never the raw `value` column.
        let mut median = median_metric();
        median.transform = Some(ValueTransform {
            clamp_max: Some(100.0),
            ..ValueTransform::default()
        });
        let hist = compile_histogram_query(&median, &request(), &[]);
        assert!(
            hist.sql
                .contains("if((value) IS NULL, NULL, least(100.0, value))")
                && hist.sql.contains("AS event_value"),
            "histogram must transform into a distinct event_value alias: {}",
            hist.sql
        );
        assert!(
            hist.sql.contains("min(event_value) OVER")
                && hist.sql.contains("max(event_value) OVER"),
            "histogram lo/hi must derive from the transformed alias: {}",
            hist.sql
        );
        assert_eq!(hist.sql.matches('?').count(), hist.params.len());
        let period = compile_period_batch_query(&[&def], &request(), &[]);
        assert!(
            period
                .sql
                .contains("if((m0) IS NULL, NULL, least(100.0, m0))"),
            "batch transform must wrap the item alias: {}",
            period.sql
        );
        assert_eq!(period.sql.matches('?').count(), period.params.len());
        let peer = compile_peer_batch_query(
            &[&def],
            &request(),
            "org_unit",
            PeerPopulation::DeclaredCohort,
            &[],
        );
        assert!(
            peer.sql.contains("if((metric_values.m0) IS NULL, NULL,"),
            "peer carry must transform before percentiles: {}",
            peer.sql
        );
        assert_eq!(peer.sql.matches('?').count(), peer.params.len());
    }
}
