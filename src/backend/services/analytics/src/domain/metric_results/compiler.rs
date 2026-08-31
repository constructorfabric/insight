use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;

use serde::Deserialize;

use super::batch::{
    PeerPopulation, ResolvedGroupLimit, peer_aliases, period_alias, period_compare_alias,
};
use super::validation::{
    DateWindow, HISTOGRAM_BINS, ValidatedDimensionFilter, ValidatedEntitySelection,
    ValidatedMetricResultsRequest, query_row_limit,
};
use super::view::Bucket;
use crate::domain::metric_definitions::{
    AliasCollapse, CohortSource, ComputationSpec, MetricDefinition, MetricInput, ObservationSource,
    RatioDenominatorAggregation,
};

pub(crate) const UNKNOWN_DIMENSION_VALUE: &str = "__unknown__";
pub(crate) const UNKNOWN_DIMENSION_LABEL: &str = "Unknown";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryBucket {
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl From<Bucket> for QueryBucket {
    fn from(bucket: Bucket) -> Self {
        match bucket {
            Bucket::Day => Self::Day,
            Bucket::Week => Self::Week,
            Bucket::Month => Self::Month,
        }
    }
}

/// The live email → person map every person-entity read resolves through.
pub(crate) const PERSON_MAP_RELATION: &str = "identity.person_map";

/// The live account → person binding, consulted BEFORE the email map: the
/// source's own key for the person, so it survives an empty or private
/// profile email and an address change.
pub(crate) const ACCOUNT_ASSIGNMENT_RELATION: &str = "identity.account_assignment";

/// The reserved person meaning "not a human" (bots, CI, service accounts).
/// An account bound to it TERMINATES resolution: the row attributes to
/// nobody and never falls through to the email map.
const EXCLUDED_PERSON_ID: &str = "ffffffff-ffff-ffff-ffff-ffffffffffff";

/// Columns a resolved observation subquery re-exposes to the query above it.
/// `entity_id` is absent: the subquery replaces it with the canonical person id,
/// so every outer clause reads unchanged.
const RESOLVED_OBSERVATION_COLUMNS: &str = "tenant_id, entity_type, source_key, measure_key, metric_date, observed_at, value, \
     subject_key, dimensions";

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
    /// The same reading over the comparison window; `None` when none was asked
    /// for.
    #[serde(default)]
    pub compare_to: Option<f64>,
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

/// One observed (dimension tuple, bin) pair plus the tuple's exact value
/// bounds, pooled over all selected entities' events — the dimensioned
/// counterpart of [`HistogramQueryRow`], with the dimension value/label
/// aliases arriving through `extra` like every dimensioned row shape.
#[derive(Debug, Deserialize)]
pub struct PooledHistogramQueryRow {
    pub bin_idx: u32,
    pub group_lo: f64,
    pub group_hi: f64,
    #[serde(default, deserialize_with = "optional_u64")]
    pub bin_count: Option<u64>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

pub(crate) fn compile_period_batch_query(
    defs: &[&MetricDefinition],
    req: &ValidatedMetricResultsRequest,
    filters: &[ValidatedDimensionFilter],
) -> CompiledQuery {
    let mut params = Vec::new();
    // With a comparison window the outer WHERE spans both, so each aggregate —
    // the primary one included — carries its own window term. Without one the
    // outer WHERE alone scopes the dates and the SQL is unchanged.
    let mut selects = item_value_selects(defs, &mut params, period_alias, primary_window(req));
    if let Some(compare_to) = req.compare_to {
        selects.push_str(&item_value_selects(
            defs,
            &mut params,
            period_compare_alias,
            Some(compare_to),
        ));
    }
    let read = batch_resolved_observation_from(defs, req, ScanScope::WithComparison, &mut params);
    let metric_scope =
        shared_observation_where_within(defs, req, filters, ScanScope::WithComparison, &mut params);
    let entity_scope = read.entity_scope(req, &mut params);
    let observation_table = &read.from;
    let limit = query_row_limit();
    let inner = format!(
        r"
        SELECT
            entity_id{selects}
        FROM {observation_table}
        WHERE {metric_scope}{entity_scope}
        GROUP BY entity_id
        LIMIT {limit}
        "
    );
    let sql = transformed_batch(defs, inner, req.compare_to.is_some());
    CompiledQuery { sql, params }
}

/// The window the primary value column answers: `None` — and so no window term
/// at all — until a comparison window makes the outer WHERE span two ranges.
fn primary_window(req: &ValidatedMetricResultsRequest) -> Option<DateWindow> {
    req.compare_to.map(|_| DateWindow {
        from: req.from,
        to: req.to,
    })
}

/// Which ranges a read scans.
///
/// INVARIANT: this is a property of the VIEW, not of the request. Only `period`
/// and `breakdown` answer the comparison window, and they scope each aggregate
/// to its own range. Every other view answers over the primary period alone and
/// its aggregates carry NO window term — widening its scan would silently fold
/// both ranges into one number.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanScope {
    PrimaryOnly,
    WithComparison,
}

impl ScanScope {
    fn windows(self, req: &ValidatedMetricResultsRequest) -> Vec<DateWindow> {
        let primary = DateWindow {
            from: req.from,
            to: req.to,
        };
        match (self, req.compare_to) {
            (Self::WithComparison, Some(compare_to)) => vec![primary, compare_to],
            (Self::WithComparison, None) | (Self::PrimaryOnly, _) => vec![primary],
        }
    }
}

/// The date predicate a read scans: the primary period, and the comparison
/// window when the view is one that answers it.
///
/// INVARIANT: a disjunction, never `min(from)..max(to)`. The envelope form
/// reads every day BETWEEN two far-apart windows and lets the conditional
/// aggregates discard it, which is the cost this whole request shape exists to
/// avoid. A single-range request yields the bare term it always had, so its
/// compiled SQL is unchanged.
fn scan_window_predicate(
    req: &ValidatedMetricResultsRequest,
    column: &str,
    separator: &str,
    scope: ScanScope,
    params: &mut Vec<String>,
) -> String {
    let mut terms = Vec::with_capacity(2);
    for window in scope.windows(req) {
        params.push(window.from.to_string());
        params.push(window.to.to_string());
        terms.push(format!(
            "{column} >= toDate(?){separator}AND {column} <= toDate(?)"
        ));
    }
    match terms.len() {
        1 => terms.into_iter().next().unwrap_or_default(),
        _ => format!(
            "({})",
            terms
                .into_iter()
                .map(|term| format!("({term})"))
                .collect::<Vec<_>>()
                .join(" OR ")
        ),
    }
}

/// The SQL term scoping one conditional aggregate to a window, with its bounds
/// pushed in textual order. Empty for the unwindowed form.
fn window_term(window: Option<DateWindow>, params: &mut Vec<String>) -> String {
    match window {
        None => String::new(),
        Some(window) => {
            params.push(window.from.to_string());
            params.push(window.to.to_string());
            " AND metric_date >= toDate(?) AND metric_date <= toDate(?)".to_owned()
        }
    }
}

pub(crate) fn compile_timeseries_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    bucket: QueryBucket,
    dimensions: &[String],
    filters: &[ValidatedDimensionFilter],
    group_limit: Option<&ResolvedGroupLimit>,
) -> CompiledQuery {
    if let Some(group_limit) = group_limit {
        return compile_capped_timeseries_query(def, req, bucket, dimensions, filters, group_limit);
    }
    let mut params = grouped_value_params(def);
    let read = single_resolved_observation_from(def, req, ScanScope::PrimaryOnly, &mut params);
    params.extend(metric_where_params(def, req));
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_scope = read.entity_scope(req, &mut params);
    let observation_table = &read.from;
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
          {filter_where}{entity_scope}
        GROUP BY GROUPING SETS (({bucket_group}), ({total_group}))
        ORDER BY entity_id, is_total, bucket_start
        LIMIT {limit}
        ",
        metric_where = metric_where(def, req.enforce_tenant_scope),
    );
    let sql = transformed_single(def, inner);
    CompiledQuery { sql, params }
}

pub(crate) fn compile_report_timeseries_query(
    def: &MetricDefinition,
    tenant_id: uuid::Uuid,
    entity: ValidatedEntitySelection,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
    enforce_tenant_scope: bool,
    bucket: QueryBucket,
) -> CompiledQuery {
    let request = ValidatedMetricResultsRequest {
        tenant_id,
        entity,
        from,
        to,
        metrics: Vec::new(),
        enforce_tenant_scope,
        compare_to: None,
    };

    compile_timeseries_query(def, &request, bucket, &[], &[], None)
}

pub(crate) fn compile_group_ranking_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    dimensions: &[String],
    filters: &[ValidatedDimensionFilter],
    count: usize,
) -> CompiledQuery {
    let mut params = grouped_value_params(def);
    let read = single_resolved_observation_from(def, req, ScanScope::PrimaryOnly, &mut params);
    params.extend(metric_where_params(def, req));
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_scope = read.entity_scope(req, &mut params);
    let observation_table = &read.from;
    let (dim_select, dim_group, dim_order) = ranking_dimension_select_group(dimensions);
    let value_expr = grouped_value_expr(def);
    let inner = format!(
        r"
        SELECT
            {dim_select},
            {value_expr} AS value
        FROM {observation_table}
        WHERE {metric_where}
          {filter_where}{entity_scope}
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
    bucket: QueryBucket,
    dimensions: &[String],
    filters: &[ValidatedDimensionFilter],
    group_limit: &ResolvedGroupLimit,
) -> CompiledQuery {
    let mut params = Vec::new();
    let read = single_resolved_observation_from(def, req, ScanScope::PrimaryOnly, &mut params);
    params.extend(metric_where_params(def, req));
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_scope = read.entity_scope(req, &mut params);
    let observation_table = &read.from;
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
              {filter_where}{entity_scope}
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
    let mut params = Vec::new();
    let mut value_selects = String::new();
    for (staged, presence_alias, window) in breakdown_columns(req) {
        let expr = grouped_value_expr_within(def, window, &mut params);
        let _ = write!(
            value_selects,
            ",
            {expr} AS {staged}"
        );
        // Presence is its own column because the value cannot carry it: a
        // ratio over a group that IS in the window reads NULL whenever its
        // denominator is zero, which is indistinguishable from a group the
        // window never had. A standalone request over that window returns the
        // first and omits the second, so the projection needs both facts.
        let (Some(window), Some(presence_alias)) = (window, presence_alias) else {
            continue;
        };
        let presence = window_presence_expr(window, &mut params);
        let _ = write!(
            value_selects,
            ",
            {presence} AS {presence_alias}"
        );
    }
    let read = single_resolved_observation_from(def, req, ScanScope::WithComparison, &mut params);
    let metric_where = metric_where_scanned(def, req, &mut params);
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_scope = read.entity_scope(req, &mut params);
    let observation_table = &read.from;
    let (dim_select, dim_group) = dimension_select_group(dimensions);
    let (source_select, source_group) = hidden_source_context(dimensions);
    let group = if dim_group.is_empty() {
        "entity_id".to_owned()
    } else {
        format!("entity_id, {dim_group}")
    };
    let group = format!("{group}{source_group}");
    let limit = query_row_limit();
    let inner = format!(
        r"
        SELECT
            entity_id{dim_select}{source_select}{value_selects}
        FROM {observation_table}
        WHERE {metric_where}
          {filter_where}{entity_scope}
        GROUP BY {group}
        ORDER BY entity_id
        LIMIT {limit}
        "
    );
    let sql = if req.compare_to.is_none() {
        transformed_single(def, inner)
    } else {
        projected_breakdown_columns(def, &inner)
    };
    CompiledQuery { sql, params }
}

/// The value column per window a breakdown computes, with the staged name it
/// is aliased to inside the query and the presence flag beside it.
///
/// INVARIANT: a windowed breakdown must NOT alias any aggregate `value`. The
/// aggregates read a column of that name, and a sibling's argument resolves to
/// the alias instead of the column — which ClickHouse rejects outright as an
/// aggregate inside an aggregate. The wire names are put back by
/// `projected_breakdown_columns`.
type BreakdownColumn = (&'static str, Option<&'static str>, Option<DateWindow>);

fn breakdown_columns(req: &ValidatedMetricResultsRequest) -> Vec<BreakdownColumn> {
    let Some(compare_to) = req.compare_to else {
        return vec![("value", None, None)];
    };
    let primary = DateWindow {
        from: req.from,
        to: req.to,
    };
    vec![
        (STAGED_VALUE, Some(PRESENT), Some(primary)),
        (STAGED_COMPARE, Some(PRESENT_COMPARE), Some(compare_to)),
    ]
}

const STAGED_VALUE: &str = "staged_value";
const STAGED_COMPARE: &str = "staged_compare";
pub(crate) const PRESENT: &str = "present";
pub(crate) const PRESENT_COMPARE: &str = "present_compare";
pub(crate) const VALUE_COMPARE: &str = "value_compare";

/// Whether a group has any row inside one window — the fact a standalone
/// request over that window expresses by emitting the group or not.
fn window_presence_expr(window: DateWindow, params: &mut Vec<String>) -> String {
    params.push(window.from.to_string());
    params.push(window.to.to_string());
    "countIf(metric_date >= toDate(?) AND metric_date <= toDate(?)) > 0".to_owned()
}

/// `metric_where` with its own placeholders bound, so the scanned dates can be
/// a disjunction whose arity depends on the request.
fn metric_where_scanned(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    params: &mut Vec<String>,
) -> String {
    let (source_key, measures) = match &def.spec {
        ComputationSpec::Sum { value }
        | ComputationSpec::Median { value }
        | ComputationSpec::Percentile { value, .. }
        | ComputationSpec::Stddev { value }
        | ComputationSpec::DistinctCount { value } => {
            (value.source_key.clone(), vec![value.measure_key.clone()])
        }
        ComputationSpec::Ratio {
            numerator,
            denominator,
            ..
        } => (
            numerator.source_key.clone(),
            vec![
                numerator.measure_key.clone(),
                denominator.measure_key.clone(),
            ],
        ),
    };
    let tenant = tenant_predicate(req.enforce_tenant_scope);
    params.push(req.tenant_id.to_string());
    params.push(source_key);
    params.push(req.entity.entity_type().to_owned());
    let scan = scan_window_predicate(req, "metric_date", " ", ScanScope::WithComparison, params);
    let measure_predicate = if measures.len() == 1 {
        "measure_key = ?"
    } else {
        "measure_key IN (?, ?)"
    };
    for measure in measures {
        params.push(measure);
    }
    format!("{tenant} AND source_key = ? AND entity_type = ? AND {scan} AND {measure_predicate}")
}

/// Rename the staged columns to their wire names, applying the value transform
/// on the way — the projection stage `transformed_single` performs for an
/// uncompared breakdown, over both columns.
fn projected_breakdown_columns(def: &MetricDefinition, inner: &str) -> String {
    let mut selects = String::new();
    for (staged, wire) in [(STAGED_VALUE, "value"), (STAGED_COMPARE, VALUE_COMPARE)] {
        let expr = transformed(def, staged.to_owned());
        let _ = write!(
            selects,
            ",
            {expr} AS {wire}"
        );
    }
    let excluded = format!("{STAGED_VALUE}, {STAGED_COMPARE}");
    format!(
        r"
        SELECT
            * EXCEPT ({excluded}){selects}
        FROM ({inner})
        "
    )
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
    let mut params = grouped_value_params(def);
    let read = single_resolved_observation_from(def, req, ScanScope::PrimaryOnly, &mut params);
    params.extend(metric_where_params(def, req));
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_scope = read.entity_scope(req, &mut params);
    let observation_table = &read.from;
    let (dim_select, dim_group, dim_order) = ranking_dimension_select_group(dimensions);
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
          {filter_where}{entity_scope}
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
    let mut params = Vec::new();
    let read = single_resolved_observation_from(def, req, ScanScope::PrimaryOnly, &mut params);
    params.extend(metric_where_params(def, req));
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_scope = read.entity_scope(req, &mut params);
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
    let observation_table = &read.from;
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
              {filter_where}{entity_scope}
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

/// `grouped_value_expr` scoped to one window, pushing its own placeholders in
/// textual order — the SELECT-list arity now varies per request, so the ratio
/// arm can no longer lean on `metric_params` leading with a fixed pair.
/// `window: None` reproduces the unwindowed expression exactly.
fn grouped_value_expr_within(
    def: &MetricDefinition,
    window: Option<DateWindow>,
    params: &mut Vec<String>,
) -> String {
    match &def.spec {
        ComputationSpec::Sum { .. } => {
            let window = window_term(window, params);
            format!("sumIf(value, value IS NOT NULL{window})")
        }
        ComputationSpec::Ratio {
            numerator,
            denominator,
            scale,
            denominator_aggregation,
        } => {
            params.push(numerator.measure_key.clone());
            let numerator_window = window_term(window, params);
            params.push(denominator.measure_key.clone());
            let denominator_window = window_term(window, params);
            let denominator = ratio_denominator_expr(
                *denominator_aggregation,
                &format!("measure_key = ? AND value IS NOT NULL{denominator_window}"),
                &format!("measure_key = ? AND subject_key IS NOT NULL{denominator_window}"),
            );
            format!(
                "{scale} * sumIfOrNull(value, measure_key = ? AND value IS NOT NULL{numerator_window}) / nullIf({denominator}, 0)"
            )
        }
        ComputationSpec::Median { .. } => {
            let window = window_term(window, params);
            format!("quantileExactIf(0.5)(value, value IS NOT NULL{window})")
        }
        ComputationSpec::Percentile { q, .. } => {
            let window = window_term(window, params);
            format!("quantileExactIf({q})(value, value IS NOT NULL{window})")
        }
        ComputationSpec::Stddev { .. } => {
            let window = window_term(window, params);
            format!("stddevSampIf(value, value IS NOT NULL{window})")
        }
        ComputationSpec::DistinctCount { .. } => {
            let window = window_term(window, params);
            format!("toFloat64(uniqExactIf(subject_key, subject_key IS NOT NULL{window}))")
        }
    }
}

// Deterministic fixed-width binning over each entity's exact [min, max]:
// pure arithmetic over exact aggregates, so identical data always yields
// identical bins (the adaptive `histogram()` aggregate is merge-order
// dependent). `least(max_bin, …)` closes the last bin at the maximum; a
// degenerate range (all values identical) maps everything to bin 0, which
// the builder renders as one [v, v] bin. Validation guarantees the metric is
// a median or percentile (single-measure predicate), so
// metric_where/metric_params fit.
pub(crate) fn compile_histogram_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    filters: &[ValidatedDimensionFilter],
) -> CompiledQuery {
    let mut params = grouped_value_params(def);
    let read = single_resolved_observation_from(def, req, ScanScope::PrimaryOnly, &mut params);
    params.extend(metric_where_params(def, req));
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_scope = read.entity_scope(req, &mut params);
    let observation_table = &read.from;
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
              {filter_where}{entity_scope}
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

// The pooled counterpart of `compile_histogram_query`: same deterministic
// fixed-width binning, but the partition (and the [min, max] bounds) is the
// dimension tuple instead of the entity — all selected entities' events pool
// into one distribution per tuple, mirroring how rollup drops the entity
// grain. Grouping by (value, label) alias pairs matches the breakdown /
// timeseries dimension shape. Group cardinality is unbounded like an
// uncapped rollup's, so the query carries the same row limit.
pub(crate) fn compile_pooled_histogram_query(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    dimensions: &[String],
    filters: &[ValidatedDimensionFilter],
) -> CompiledQuery {
    let mut params = grouped_value_params(def);
    let read = single_resolved_observation_from(def, req, ScanScope::PrimaryOnly, &mut params);
    params.extend(metric_where_params(def, req));
    let filter_where = dimension_filter_where(filters, &mut params);
    let entity_scope = read.entity_scope(req, &mut params);
    let observation_table = &read.from;
    let (dim_select, dim_group) = dimension_select_group(dimensions);
    let bins = HISTOGRAM_BINS;
    let max_bin = HISTOGRAM_BINS - 1;
    let limit = query_row_limit();
    let sql = format!(
        r"
        WITH raw_events AS (
            SELECT
                assumeNotNull({event_value}) AS event_value{dim_select}
            FROM {observation_table}
            WHERE {metric_where}
              {filter_where}{entity_scope}
              AND value IS NOT NULL
        ),
        events AS (
            SELECT
                *,
                min(event_value) OVER (PARTITION BY {dim_group}) AS group_lo,
                max(event_value) OVER (PARTITION BY {dim_group}) AS group_hi
            FROM raw_events
        )
        SELECT
            {dim_group},
            if(
                events.group_hi = events.group_lo,
                0,
                toUInt32(least({max_bin}, toInt64(floor(
                    (events.event_value - events.group_lo) * {bins} / (events.group_hi - events.group_lo)
                ))))
            ) AS bin_idx,
            any(events.group_lo) AS group_lo,
            any(events.group_hi) AS group_hi,
            toUInt64(count()) AS bin_count
        FROM events
        GROUP BY {dim_group}, bin_idx
        ORDER BY {dim_group}, bin_idx
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
    let value_selects = item_value_selects(defs, &mut params, period_alias, None);
    let collapses = batch_alias_collapses(defs);
    let observation_table = resolved_observation_from(
        batch_observation_source(defs),
        &PersonScope::CohortMembers(req),
        &collapses,
        ScanScope::PrimaryOnly,
        &mut params,
    )
    .from;
    let metric_scope =
        shared_observation_where_within(defs, req, filters, ScanScope::PrimaryOnly, &mut params);

    let entity_id_params = placeholders(req.entity.len());
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
    let value_selects = item_value_selects(defs, &mut params, period_alias, None);
    let collapses = batch_alias_collapses(defs);
    let observation_table = resolved_observation_from(
        batch_observation_source(defs),
        &PersonScope::TenantWide(req),
        &collapses,
        ScanScope::PrimaryOnly,
        &mut params,
    )
    .from;
    let metric_scope =
        shared_observation_where_within(defs, req, filters, ScanScope::PrimaryOnly, &mut params);

    let entity_id_params = placeholders(req.entity.len());
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
    alias: impl Fn(usize) -> String,
    window: Option<DateWindow>,
) -> String {
    let mut selects = String::new();
    for (item_index, def) in defs.iter().enumerate() {
        let expr = item_value_expr(def, params, window);
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
fn item_value_expr(
    def: &MetricDefinition,
    params: &mut Vec<String>,
    window: Option<DateWindow>,
) -> String {
    match &def.spec {
        ComputationSpec::Sum { value } => {
            params.push(value.source_key.clone());
            params.push(value.measure_key.clone());
            let window = window_term(window, params);
            format!(
                "sumIfOrNull(value, source_key = ? AND measure_key = ? AND value IS NOT NULL{window})"
            )
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
            let numerator_window = window_term(window, params);
            params.push(numerator.source_key.clone());
            params.push(denominator.measure_key.clone());
            let denominator_window = window_term(window, params);
            let denominator = ratio_denominator_expr(
                *denominator_aggregation,
                &format!(
                    "source_key = ? AND measure_key = ? AND value IS NOT NULL{denominator_window}"
                ),
                &format!(
                    "source_key = ? AND measure_key = ? AND subject_key IS NOT NULL{denominator_window}"
                ),
            );
            format!(
                "{scale} * sumIfOrNull(value, source_key = ? AND measure_key = ? AND value IS NOT NULL{numerator_window}) / nullIf({denominator}, 0)"
            )
        }
        ComputationSpec::Median { value } => {
            // OrNull so an entity present in the batch (via another measure)
            // but with no rows for this measure comes back NULL, not 0 — the
            // builder never zero-fills medians (honest-null).
            params.push(value.source_key.clone());
            params.push(value.measure_key.clone());
            let window = window_term(window, params);
            format!(
                "quantileExactIfOrNull(0.5)(value, source_key = ? AND measure_key = ? AND value IS NOT NULL{window})"
            )
        }
        ComputationSpec::Percentile { value, q } => {
            // Honest-null like Median — same event-grain shape, different point
            // on the distribution.
            params.push(value.source_key.clone());
            params.push(value.measure_key.clone());
            let window = window_term(window, params);
            format!(
                "quantileExactIfOrNull({q})(value, source_key = ? AND measure_key = ? AND value IS NOT NULL{window})"
            )
        }
        ComputationSpec::Stddev { value } => {
            // Honest-null like Median; sample stddev, so a single observation
            // reads NULL (no spread is measurable), never a fabricated 0.
            params.push(value.source_key.clone());
            params.push(value.measure_key.clone());
            let window = window_term(window, params);
            format!(
                "stddevSampIfOrNull(value, source_key = ? AND measure_key = ? AND value IS NOT NULL{window})"
            )
        }
        ComputationSpec::DistinctCount { value } => {
            // OrNull like sum: an entity present via another measure but with
            // no rows for this one comes back NULL, not 0, so it never enters
            // a peer pool as a fabricated observation. The builder zero-fills
            // distinct counts (0 distinct subjects is a genuine zero) exactly
            // as it does sums. Counts distinct `subject_key`, not `value`.
            params.push(value.source_key.clone());
            params.push(value.measure_key.clone());
            let window = window_term(window, params);
            // toFloat64 so the wide column is Float64, not a JSON-quoted
            // UInt64 (uniqExact's native type) the f64 row decoder rejects.
            // OrNull is preserved through the cast (NULL stays NULL).
            format!(
                "toFloat64(uniqExactIfOrNull(subject_key, source_key = ? AND measure_key = ? AND subject_key IS NOT NULL{window}))"
            )
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

// Batch variant: re-projects each transformed item column by alias, the
// comparison window's column included.
fn transformed_batch(defs: &[&MetricDefinition], inner: String, compared: bool) -> String {
    if defs.iter().all(|def| def.transform.is_none()) {
        return inner;
    }
    let mut selects = String::new();
    for (item_index, def) in defs.iter().enumerate() {
        for value in [
            Some(period_alias(item_index)),
            compared.then(|| period_compare_alias(item_index)),
        ]
        .into_iter()
        .flatten()
        {
            let expr = transformed(def, value.clone());
            let _ = write!(selects, ", {expr} AS {value}");
        }
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

/// `shared_observation_where` over an explicit scan range: a windowed batch
/// scans the union of its windows once and lets each conditional aggregate
/// pick its own out of it.
fn shared_observation_where_within(
    defs: &[&MetricDefinition],
    req: &ValidatedMetricResultsRequest,
    filters: &[ValidatedDimensionFilter],
    scope: ScanScope,
    params: &mut Vec<String>,
) -> String {
    params.push(req.tenant_id.to_string());
    params.push(req.entity.entity_type().to_owned());
    let scan = scan_window_predicate(req, "metric_date", " ", scope, params);
    let pairs = measure_pairs(defs);
    for (source_key, measure_key) in &pairs {
        params.push(source_key.clone());
        params.push(measure_key.clone());
    }
    let pair_placeholders = vec!["(?, ?)"; pairs.len()].join(", ");
    let tenant = tenant_predicate(req.enforce_tenant_scope);
    let mut where_clause = format!(
        "{tenant} AND entity_type = ? AND {scan} AND (source_key, measure_key) IN ({pair_placeholders})"
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

fn batch_observation_source<'a>(defs: &'a [&MetricDefinition]) -> &'a ObservationSource {
    defs.first()
        .unwrap_or_else(|| unreachable!("batches are planned from at least one metric view"))
        .observation_source()
}

/// `FROM` target for a batch. The collapse stage is selective per measure, so
/// sharing a batch never changes a single measure's semantics.
fn batch_resolved_observation_from(
    defs: &[&MetricDefinition],
    req: &ValidatedMetricResultsRequest,
    scope: ScanScope,
    params: &mut Vec<String>,
) -> ObservationRead {
    let def = defs
        .first()
        .unwrap_or_else(|| unreachable!("batches are planned from at least one metric view"));
    let collapses = batch_alias_collapses(defs);
    resolved_observation_from(
        def.observation_source(),
        &PersonScope::Requested(req),
        &collapses,
        scope,
        params,
    )
}

fn single_resolved_observation_from(
    def: &MetricDefinition,
    req: &ValidatedMetricResultsRequest,
    scope: ScanScope,
    params: &mut Vec<String>,
) -> ObservationRead {
    let defs = [def];
    let collapses = batch_alias_collapses(&defs);
    resolved_observation_from(
        def.observation_source(),
        &PersonScope::Requested(req),
        &collapses,
        scope,
        params,
    )
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

// INVARIANT: the bucket key must be non-nullable — a custom source may
// declare `metric_date` as `Nullable(Date)`, and GROUPING SETS fills the
// totals row's absent key with NULL instead of the default date, which the
// row parser rejects. The date-range predicate has already dropped NULL
// dates, so `assumeNotNull` never sees one.
fn bucket_expr(bucket: QueryBucket) -> &'static str {
    match bucket {
        QueryBucket::Day => "assumeNotNull(metric_date)",
        QueryBucket::Week => "toStartOfWeek(assumeNotNull(metric_date), 1)",
        QueryBucket::Month => "toStartOfMonth(assumeNotNull(metric_date))",
        QueryBucket::Quarter => "toStartOfQuarter(assumeNotNull(metric_date))",
        QueryBucket::Year => "toStartOfYear(assumeNotNull(metric_date))",
    }
}

fn observation_table(source: &ObservationSource) -> String {
    source.render_from_clause()
}

/// Whether a read resolves identity while it serves, or reads a relation that
/// already carries canonical ids.
///
/// SAFETY: a custom source is tenant-authored SQL whose contract says
/// `entity_id` is already the canonical person id. Resolving it would look up an
/// id that is not an email and silently return nothing. A tenant-entity read is
/// canonical for every source: tenant evidence repeats its tenant key as the
/// entity id, so there is no person to resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntityResolution {
    QueryTime,
    Canonical,
}

impl EntityResolution {
    fn of(source: &ObservationSource, entity: &ValidatedEntitySelection) -> Self {
        match (source, entity) {
            (ObservationSource::Custom(_), _)
            | (ObservationSource::Managed(_), ValidatedEntitySelection::Tenant { .. }) => {
                Self::Canonical
            }
            (ObservationSource::Managed(_), ValidatedEntitySelection::Person { .. }) => {
                Self::QueryTime
            }
        }
    }
}

/// Which people an observation read resolves and returns.
enum PersonScope<'a> {
    /// The people the request asked about. Binds their ids (prune), the date
    /// range, then their ids again (the resolved filter) — the caller's param
    /// contract.
    Requested(&'a ValidatedMetricResultsRequest),
    /// Everyone in the peer pool, read from the `cohort` CTE the peer query
    /// defines above this subquery. Binds no person ids — a peer comparison
    /// needs the whole pool's values, and narrowing to the requested people
    /// would answer with a pool of one — only the date range, so the scan is
    /// bounded here rather than trusting predicate pushdown from above.
    CohortMembers(&'a ValidatedMetricResultsRequest),
    /// Everyone in the tenant: the whole-population peer pool. Every row that
    /// resolves to a person joins the pool; only the date range bounds the
    /// scan.
    TenantWide(&'a ValidatedMetricResultsRequest),
}

impl PersonScope<'_> {
    fn request(&self) -> &ValidatedMetricResultsRequest {
        match self {
            Self::Requested(req) | Self::CohortMembers(req) | Self::TenantWide(req) => req,
        }
    }
}

/// An observation read: where to read from, and whether the people were already
/// narrowed while resolving identity.
struct ObservationRead {
    from: String,
    resolution: EntityResolution,
}

impl ObservationRead {
    /// The entity term for a source that was NOT resolved in the `FROM`. A
    /// resolved read is already scoped to the requested people by the map
    /// lookup; a canonical one still needs the filter here, or the read would
    /// answer with everybody. Pushes its params where the term renders.
    fn entity_scope(
        &self,
        req: &ValidatedMetricResultsRequest,
        params: &mut Vec<String>,
    ) -> String {
        match self.resolution {
            EntityResolution::QueryTime => String::new(),
            EntityResolution::Canonical => match &req.entity {
                ValidatedEntitySelection::Person { .. } => {
                    params.extend(req.entity.entity_ids());
                    format!(
                        "\n          AND entity_id IN ({})",
                        placeholders(req.entity.len())
                    )
                }
                // Tenant evidence repeats its tenant key as the entity id, so
                // the row-internal equality is the whole scope — no id to bind.
                ValidatedEntitySelection::Tenant { .. } => {
                    "\n          AND entity_id = tenant_id".to_owned()
                }
            },
        }
    }
}

/// The identity store's form of an observation row's account-binding key.
///
/// INVARIANT: identity stores `source_id` as sipHash128 of the connector's raw
/// `source_id` (see the connectors' `identity_inputs` models) and
/// `identity.account_assignment` lowercases `account_id` — both expressions
/// must stay in lockstep with those minting rules, or the lookup silently
/// matches nothing.
pub(crate) fn account_source_uuid_expr(alias: &str) -> String {
    format!("toUUID(UUIDNumToString(sipHash128(coalesce({alias}.account_source_id, ''))))")
}

pub(crate) fn account_id_expr(alias: &str) -> String {
    format!("lower(trimBoth(coalesce({alias}.account_id, '')))")
}

/// The `FROM` target for an observation read, with identity resolved when the
/// source needs it.
///
/// A subquery, not a join spliced into the caller: every clause above it keeps
/// reading a bare `entity_id` that now holds the person id, so scoping,
/// `GROUP BY`, `GROUPING SETS`, window partitions and `ORDER BY` are unchanged.
///
/// Account-first, because an account binding survives an empty profile e-mail
/// and an account bound to the excluded person must TERMINATE resolution — a
/// bot's rows would otherwise fall through to whichever human its commit
/// e-mails name.
///
/// Two predicates on purpose. The prune is for the scan (`entity_id` leads the
/// sort key after tenant and source, so the e-mail lookup reads only the
/// requested parts); the filter above `resolved` is what decides, whatever the
/// prune let through.
fn resolved_observation_from(
    source: &ObservationSource,
    person_scope: &PersonScope<'_>,
    collapses: &[(&MetricInput, AliasCollapse)],
    scan: ScanScope,
    params: &mut Vec<String>,
) -> ObservationRead {
    let table = observation_table(source);
    let resolution = EntityResolution::of(source, &person_scope.request().entity);
    if resolution == EntityResolution::Canonical {
        return ObservationRead {
            from: table,
            resolution,
        };
    }

    // INVARIANT: this read pre-filters observations before the outer aggregates
    // run, so its dates must cover every range the VIEW answers: the primary
    // period alone would answer NULL for a comparison window, and both ranges
    // would make a view that answers only the primary period read rows it must
    // not see.
    let scan_of = |req: &ValidatedMetricResultsRequest, params: &mut Vec<String>| {
        format!(
            "\n          AND {}",
            scan_window_predicate(req, "obs.metric_date", "\n          ", scan, params)
        )
    };
    let (inner_where, resolved_filter) = match person_scope {
        PersonScope::Requested(req) => {
            params.extend(req.entity.entity_ids());
            let date_scope = scan_of(req, params);
            params.extend(req.entity.entity_ids());
            let person_params = placeholders(req.entity.len());
            (
                format!(
                    "(obs.entity_id IN (SELECT email FROM {PERSON_MAP_RELATION} WHERE person_id IN ({person_params})) \
                      OR coalesce(obs.account_id, '') != ''){date_scope}"
                ),
                format!("resolved_person_id IN ({person_params})"),
            )
        }
        PersonScope::CohortMembers(req) => {
            let date_scope = scan_of(req, params);
            (
                format!(
                    "(obs.entity_id IN (SELECT email FROM {PERSON_MAP_RELATION} \
                      WHERE toString(person_id) IN (SELECT entity_id FROM cohort)) \
                      OR coalesce(obs.account_id, '') != ''){date_scope}"
                ),
                "resolved_person_id IN (SELECT entity_id FROM cohort)".to_owned(),
            )
        }
        PersonScope::TenantWide(req) => {
            let date_scope =
                scan_window_predicate(req, "obs.metric_date", "\n          ", scan, params);
            (date_scope, "resolved_person_id != ''".to_owned())
        }
    };

    // Null-proof under EITHER join_use_nulls setting (queries differ): each
    // match test is non-Nullable via coalesce on a String column, and the
    // joined person_id is read only on its matched branch.
    let resolved = format!(
        r"(
        SELECT
            {columns},
            resolved_person_id AS entity_id
        FROM (
            SELECT
                {qualified},
                multiIf(
                    coalesce(account_map.account_id, '') != '',
                    if(
                        assumeNotNull(account_map.person_id) = toUUID('{EXCLUDED_PERSON_ID}'),
                        '',
                        toString(assumeNotNull(account_map.person_id))
                    ),
                    coalesce(person_map.email, '') != '',
                    toString(assumeNotNull(person_map.person_id)),
                    ''
                ) AS resolved_person_id
            FROM {table} AS obs
            LEFT JOIN {ACCOUNT_ASSIGNMENT_RELATION} AS account_map
                ON account_map.source_type = obs.account_source_type
               AND account_map.source_id = {account_source_uuid}
               AND account_map.account_id = {account_id}
            LEFT JOIN {PERSON_MAP_RELATION} AS person_map
                ON person_map.email = obs.entity_id
            WHERE {inner_where}
        ) AS resolved
        WHERE {resolved_filter}
        )",
        columns = RESOLVED_OBSERVATION_COLUMNS,
        qualified = qualified_resolved_columns("obs"),
        account_source_uuid = account_source_uuid_expr("obs"),
        account_id = account_id_expr("obs"),
    );

    ObservationRead {
        from: collapsed_observation_from(resolved, collapses),
        resolution,
    }
}

fn qualified_resolved_columns(alias: &str) -> String {
    RESOLVED_OBSERVATION_COLUMNS
        .split(", ")
        .map(|column| format!("{alias}.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Collapses one person's several source identities to the grain the warehouse
/// relation is keyed at, for the measures whose values must not be summed across
/// them.
///
/// The `GROUP BY` is the relation's own key with the person in place of the
/// source identity, so dimensions and `subject_key` survive for the aggregates
/// above and every outer predicate selects the same rows either side of it.
fn collapsed_observation_from(
    resolved: String,
    collapses: &[(&MetricInput, AliasCollapse)],
) -> String {
    let flagged = collapses
        .iter()
        .filter(|(_, collapse)| collapse.needs_pre_collapse())
        .collect::<Vec<_>>();
    if flagged.is_empty() {
        return resolved;
    }

    // INVARIANT: only flagged measures' rows are grouped; the rest pass through.
    // Grouping everything would merge an event-grain median's same-day events
    // whenever a flagged measure shares the batch.
    let mut value_expr = String::from("multiIf(");
    let mut flagged_pairs = Vec::new();
    for (input, collapse) in &flagged {
        let aggregate = collapse.aggregate_fn();
        let source_key = sql_string_literal(&input.source_key);
        let measure_key = sql_string_literal(&input.measure_key);
        let _ = write!(
            value_expr,
            "(source_key, measure_key) = ({source_key}, {measure_key}), {aggregate}(value), "
        );
        flagged_pairs.push(format!("({source_key}, {measure_key})"));
    }
    value_expr.push_str("sum(value))");
    let flagged_set = flagged_pairs.join(", ");

    let group_by = "tenant_id, entity_type, source_key, measure_key, metric_date, entity_id, \
                    dimensions, subject_key";
    format!(
        r"(
        WITH resolved_rows AS {resolved}
        SELECT
            {group_by},
            any(observed_at) AS observed_at,
            {value_expr} AS value
        FROM resolved_rows
        WHERE (source_key, measure_key) IN ({flagged_set})
        GROUP BY {group_by}
        UNION ALL
        SELECT
            {group_by},
            observed_at,
            value
        FROM resolved_rows
        WHERE (source_key, measure_key) NOT IN ({flagged_set})
        )"
    )
}

/// SAFETY: backslash first — it escapes in ClickHouse literals, so handling
/// quotes alone would let `x\\` swallow the closing quote. `?` last: the driver
/// binds by scanning the raw query text with no regard for quoting, so a `?`
/// inside a literal would take a bound parameter and shift every later one.
/// Keys are `snake_case` by CHECK constraint and by `is_column_key`, so this
/// guards a future key shape rather than a current one.
pub(crate) fn sql_string_literal(value: &str) -> String {
    format!(
        "'{}'",
        value
            .replace('\\', "\\\\")
            .replace('\'', "''")
            .replace('?', "??")
    )
}

/// Every input of a batch with its alias-collapse rule, deduplicated by
/// (source, measure) so a measure bound twice renders one `multiIf` arm.
fn batch_alias_collapses<'a>(
    defs: &'a [&MetricDefinition],
) -> Vec<(&'a MetricInput, AliasCollapse)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for def in defs {
        for input in def.spec.inputs() {
            if seen.insert((input.source_key.as_str(), input.measure_key.as_str())) {
                out.push((input, input.alias_collapse));
            }
        }
    }
    out
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
        AliasCollapse, CustomObservationSql, MetricBase, MetricDirection, MetricFormat,
        MetricInput, MetricInputRole, ObservationRelation, ObservationSource,
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
            alias_collapse: AliasCollapse::Sum,
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

    fn percentile_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(vec!["source"]),
            spec: ComputationSpec::Percentile {
                value: input(MetricInputRole::Value, "pr_cycle_hours"),
                q: 0.75,
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

        let query =
            compile_timeseries_query(&metric, &request(), QueryBucket::Week, &[], &[], None);

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

    fn tenant_binds(query: &CompiledQuery) -> usize {
        query
            .params
            .iter()
            .filter(|param| param.as_str() == TEST_TENANT_STR)
            .count()
    }

    fn request() -> ValidatedMetricResultsRequest {
        ValidatedMetricResultsRequest {
            tenant_id: TEST_TENANT,
            entity: ValidatedEntitySelection::Person {
                ids: vec![Uuid::from_u128(0xa), Uuid::from_u128(0xb)],
            },
            from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap_or_default(),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap_or_default(),
            compare_to: None,
            metrics: Vec::new(),
            enforce_tenant_scope: true,
        }
    }

    #[test]
    fn period_batch_binds_item_params_then_resolution_then_scope_and_pairs() {
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
                // identity resolution renders in the FROM, ahead of the WHERE:
                // the people asked about (the email prune), the date range it
                // narrows to, then the same people for the resolved filter
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                "2026-01-01",
                "2026-01-31",
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                // shared scope (tenant predicate leads the WHERE)
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
            ]
        );
    }

    fn windowed_request() -> ValidatedMetricResultsRequest {
        let mut req = request();
        req.compare_to = Some(DateWindow {
            from: NaiveDate::from_ymd_opt(2025, 12, 1).unwrap_or_default(),
            to: NaiveDate::from_ymd_opt(2025, 12, 31).unwrap_or_default(),
        });
        req
    }

    #[test]
    fn a_compared_period_batch_scans_each_range_and_scopes_both_columns() {
        let sum = sum_metric();
        let query = compile_period_batch_query(&[&sum], &windowed_request(), &[]);

        assert!(query.sql.contains("AS m0"));
        assert!(query.sql.contains("AS m0_compare"));
        // The scan is a disjunction of the requested ranges. The envelope form
        // — one range from the earliest `from` to the latest `to` — would read
        // every day between two far-apart windows.
        assert!(query.sql.contains(
            "(metric_date >= toDate(?) AND metric_date <= toDate(?)) OR (metric_date >= toDate(?) AND metric_date <= toDate(?))"
        ));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
        assert_eq!(
            query.params,
            vec![
                // primary column, scoped to the period
                "ai_usage",
                "accepted_lines",
                "2026-01-01",
                "2026-01-31",
                // extra window column
                "ai_usage",
                "accepted_lines",
                "2025-12-01",
                "2025-12-31",
                // the identity-resolving read: it filters observations BEFORE
                // the aggregates, so it scans both ranges and neither the gap
                // nor the primary period alone
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                "2026-01-01",
                "2026-01-31",
                "2025-12-01",
                "2025-12-31",
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                // shared scope, same two ranges
                TEST_TENANT_STR,
                "person",
                "2026-01-01",
                "2026-01-31",
                "2025-12-01",
                "2025-12-31",
                "ai_usage",
                "accepted_lines",
            ]
        );
    }

    /// One view's compiler, named for the assertion message.
    type UnwidenedCase = (
        &'static str,
        fn(&ValidatedMetricResultsRequest) -> CompiledQuery,
    );

    /// The contract says only `period` and `breakdown` answer the comparison
    /// window. Every other view's aggregates carry no window term of their own,
    /// so a widened scan would fold both ranges into one number instead — a
    /// peer target summing two months, and percentiles built on those.
    ///
    /// The check is the strongest available: for those views the compiled SQL
    /// and its bound parameters must be IDENTICAL with and without
    /// `compare_to`, down to the identity-resolving subquery.
    #[test]
    fn only_period_and_breakdown_widen_their_scan_for_a_comparison_window() {
        let sum = sum_metric();
        let mut dimensioned = sum_metric();
        dimensioned.base.allowed_dimensions = vec!["tool".to_owned()];
        let dims = vec!["tool".to_owned()];

        let unchanged: Vec<UnwidenedCase> = vec![
            ("peer (declared cohort)", |req| {
                compile_peer_batch_query(
                    &[&sum_metric()],
                    req,
                    "org_unit",
                    PeerPopulation::DeclaredCohort,
                    &[],
                )
            }),
            ("peer (tenant)", |req| {
                compile_peer_batch_query(
                    &[&sum_metric()],
                    req,
                    "org_unit",
                    PeerPopulation::Tenant,
                    &[],
                )
            }),
            ("timeseries", |req| {
                compile_timeseries_query(&sum_metric(), req, Bucket::Week.into(), &[], &[], None)
            }),
            ("rollup", |req| {
                compile_rollup_query(
                    &sum_metric(),
                    req,
                    std::slice::from_ref(&"tool".to_owned()),
                    &[],
                    None,
                )
            }),
            ("histogram", |req| {
                compile_histogram_query(&median_metric(), req, &[])
            }),
            ("ranking", |req| {
                compile_group_ranking_query(
                    &sum_metric(),
                    req,
                    std::slice::from_ref(&"tool".to_owned()),
                    &[],
                    5,
                )
            }),
        ];
        for (name, compile) in unchanged {
            let plain = compile(&request());
            let compared = compile(&windowed_request());
            assert_eq!(plain.sql, compared.sql, "{name}: SQL must not widen");
            assert_eq!(
                plain.params, compared.params,
                "{name}: bound dates must not widen"
            );
        }

        // ...and the two that do answer it must change.
        let period_plain = compile_period_batch_query(&[&sum], &request(), &[]);
        let period_compared = compile_period_batch_query(&[&sum], &windowed_request(), &[]);
        assert_ne!(period_plain.sql, period_compared.sql, "period must widen");

        let breakdown_plain = compile_breakdown_query(&dimensioned, &request(), &dims, &[]);
        let breakdown_compared =
            compile_breakdown_query(&dimensioned, &windowed_request(), &dims, &[]);
        assert_ne!(
            breakdown_plain.sql, breakdown_compared.sql,
            "breakdown must widen"
        );
    }

    #[test]
    fn an_uncompared_batch_keeps_the_single_range_form() {
        // The whole change rests on this: a request that asks for no extra
        // window compiles the SQL it always did.
        let sum = sum_metric();
        let query = compile_period_batch_query(&[&sum], &request(), &[]);

        assert!(
            query
                .sql
                .contains("AND metric_date >= toDate(?) AND metric_date <= toDate(?) AND")
        );
        assert!(!query.sql.contains(" OR ("));
        assert!(!query.sql.contains("AS m0_compare"));
    }

    #[test]
    fn a_ratio_over_windows_scopes_both_halves_of_the_fraction() {
        let ratio = ratio_metric();
        let query = compile_period_batch_query(&[&ratio], &windowed_request(), &[]);

        assert_eq!(query.sql.matches('?').count(), query.params.len());
        assert_eq!(
            query.params,
            vec![
                "ai_usage",
                "accepted_edit_actions",
                "2026-01-01",
                "2026-01-31",
                "ai_usage",
                "tool_use_offered",
                "2026-01-01",
                "2026-01-31",
                "ai_usage",
                "accepted_edit_actions",
                "2025-12-01",
                "2025-12-31",
                "ai_usage",
                "tool_use_offered",
                "2025-12-01",
                "2025-12-31",
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                "2026-01-01",
                "2026-01-31",
                "2025-12-01",
                "2025-12-31",
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                TEST_TENANT_STR,
                "person",
                "2026-01-01",
                "2026-01-31",
                "2025-12-01",
                "2025-12-31",
                "ai_usage",
                "accepted_edit_actions",
                "ai_usage",
                "tool_use_offered",
            ]
        );
    }

    #[test]
    fn a_compared_breakdown_emits_a_value_and_a_presence_column_per_window() {
        let sum = sum_metric();
        let query = compile_breakdown_query(
            &sum,
            &windowed_request(),
            std::slice::from_ref(&"tool".to_owned()),
            &[],
        );

        assert!(query.sql.contains("AS value"));
        assert!(query.sql.contains("AS value_compare"));
        // Presence rides its own column: a group's value cannot express it,
        // because a ratio over a group that IS in the window reads NULL when
        // its denominator is zero.
        assert!(query.sql.contains("AS present"));
        assert!(query.sql.contains("AS present_compare"));
        assert_eq!(
            query
                .sql
                .matches("countIf(metric_date >= toDate(?) AND metric_date <= toDate(?)) > 0")
                .count(),
            2,
            "one presence column per window, the primary included"
        );
        // The aggregates stage under names that cannot collide with the
        // observation's own `value` column — see `breakdown_aliases`.
        assert!(query.sql.contains("AS staged_value"));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
        assert_eq!(
            query.params,
            vec![
                // primary value, then primary presence
                "2026-01-01",
                "2026-01-31",
                "2026-01-01",
                "2026-01-31",
                // the extra window's value, then its presence
                "2025-12-01",
                "2025-12-31",
                "2025-12-01",
                "2025-12-31",
                // identity-resolving read over both ranges
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                "2026-01-01",
                "2026-01-31",
                "2025-12-01",
                "2025-12-31",
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                // metric scope, same two ranges
                TEST_TENANT_STR,
                "ai_usage",
                "person",
                "2026-01-01",
                "2026-01-31",
                "2025-12-01",
                "2025-12-31",
                "accepted_lines",
            ]
        );
    }

    #[test]
    fn an_uncompared_breakdown_carries_no_presence_column() {
        let sum = sum_metric();
        let query = compile_breakdown_query(
            &sum,
            &request(),
            std::slice::from_ref(&"tool".to_owned()),
            &[],
        );

        assert!(!query.sql.contains("present"));
        assert!(!query.sql.contains("staged_"));
        assert!(!query.sql.contains(" OR ("));
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
                    alias_collapse: AliasCollapse::Sum,
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
        let ts = compile_timeseries_query(&def, &request(), QueryBucket::Day, &[], &[], None);
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
                // identity resolution renders in the FROM, ahead of the WHERE:
                // prune, dates, then the resolved filter
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                "2026-01-01",
                "2026-01-31",
                "00000000-0000-0000-0000-00000000000a",
                "00000000-0000-0000-0000-00000000000b",
                TEST_TENANT_STR,
                "person",
                "2026-01-01",
                "2026-01-31",
                "ai_usage",
                "accepted_edit_actions",
                "ai_usage",
                "tool_use_offered",
            ]
        );
    }

    #[test]
    fn tenant_is_bound_from_context_once_per_contract_read() {
        let sum = sum_metric();

        let ts = compile_timeseries_query(&sum, &request(), QueryBucket::Day, &[], &[], None);
        assert!(ts.sql.contains("WHERE tenant_id = ?"), "timeseries read");
        assert_eq!(tenant_binds(&ts), 1);
        assert_eq!(ts.sql.matches('?').count(), ts.params.len());

        let rank = compile_group_ranking_query(&sum, &request(), &["tool".to_owned()], &[], 5);
        assert!(rank.sql.contains("WHERE tenant_id = ?"), "ranking read");
        assert_eq!(tenant_binds(&rank), 1);

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
        assert_eq!(tenant_binds(&peer), 3);
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
        let ts = compile_timeseries_query(&sum, &req, QueryBucket::Day, &[], &[], None);
        assert!(
            ts.sql.contains("(tenant_id = ? OR 1 = 1)"),
            "timeseries uses the bypass term"
        );
        assert!(
            !ts.sql.contains("WHERE tenant_id = ?"),
            "no exact-match tenant term when bypassed"
        );
        assert_eq!(ts.sql.matches('?').count(), ts.params.len());
        assert_eq!(tenant_binds(&ts), 1);

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
            (QueryBucket::Day, "assumeNotNull(metric_date)"),
            (
                QueryBucket::Week,
                "toStartOfWeek(assumeNotNull(metric_date), 1)",
            ),
            (
                QueryBucket::Month,
                "toStartOfMonth(assumeNotNull(metric_date))",
            ),
            (
                QueryBucket::Quarter,
                "toStartOfQuarter(assumeNotNull(metric_date))",
            ),
            (
                QueryBucket::Year,
                "toStartOfYear(assumeNotNull(metric_date))",
            ),
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
                QueryBucket::Week,
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
            QueryBucket::Day,
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
            QueryBucket::Day,
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
                // the pool read bounds its own dates inside the resolving FROM
                "2026-01-01",
                "2026-01-31",
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
                // the tenant-wide read binds only the date range: the pool is
                // every row that resolves to a person
                "2026-01-01",
                "2026-01-31",
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
        assert!(!query.sql.contains("coalesce(metric_values"));
        assert!(!query.sql.contains("coalesce(peer."));
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
        let (sum, median, ratio, distinct, percentile) = (
            sum_metric(),
            median_metric(),
            ratio_metric(),
            distinct_count_metric(),
            percentile_metric(),
        );
        for query in [
            compile_period_batch_query(
                &[&sum, &median, &ratio, &distinct, &percentile],
                &request(),
                &[],
            ),
            compile_peer_batch_query(
                &[&sum, &median, &ratio, &distinct, &percentile],
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
        let ts = compile_timeseries_query(
            &median_metric(),
            &request(),
            QueryBucket::Week,
            &[],
            &[],
            None,
        );
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
    fn percentile_batches_as_a_leveled_quantile_ornull_column() {
        // A percentile metric is a median at level p/100: same OrNull
        // honest-null batching, same placeholder/param lockstep.
        for query in [
            compile_period_batch_query(&[&percentile_metric()], &request(), &[]),
            compile_peer_batch_query(
                &[&percentile_metric()],
                &request(),
                "org_unit",
                PeerPopulation::DeclaredCohort,
                &[],
            ),
        ] {
            assert!(
                query.sql.contains(
                    "quantileExactIfOrNull(0.75)(value, source_key = ? AND measure_key = ?"
                ),
                "percentile must batch as an OrNull quantile column at its level"
            );
            assert_eq!(query.sql.matches('?').count(), query.params.len());
        }
    }

    #[test]
    fn percentile_single_views_and_histogram_use_the_declared_level() {
        let ts = compile_timeseries_query(
            &percentile_metric(),
            &request(),
            QueryBucket::Week,
            &[],
            &[],
            None,
        );
        assert!(
            ts.sql
                .contains("quantileExactIf(0.75)(value, value IS NOT NULL)")
        );
        let bd = compile_breakdown_query(
            &percentile_metric(),
            &request(),
            &["source".to_owned()],
            &[],
        );
        assert!(
            bd.sql
                .contains("quantileExactIf(0.75)(value, value IS NOT NULL)")
        );
        // Histograms bin raw events, so the level never appears there — the
        // query must still compile with the single-measure predicate shape.
        let hist = compile_histogram_query(&percentile_metric(), &request(), &[]);
        assert_eq!(hist.sql.matches('?').count(), hist.params.len());
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
        let ts = compile_timeseries_query(
            &ratio_metric(),
            &request(),
            QueryBucket::Week,
            &[],
            &[],
            None,
        );
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
            QueryBucket::Week,
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
        assert_eq!(query.sql.matches("JOIN").count(), 2);
        assert!(query.sql.contains("JOIN identity.account_assignment"));
        assert!(query.sql.contains("JOIN identity.person_map"));
        // Deterministic arithmetic only — never the adaptive aggregate.
        assert!(!query.sql.contains("histogram("));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn pooled_histogram_query_bins_per_dimension_tuple_without_entity_grain() {
        let query = compile_pooled_histogram_query(
            &percentile_metric(),
            &request(),
            &["source".to_owned()],
            &[],
        );
        // Bounds and bins partition/group by the dimension aliases — the
        // entity only scopes which rows enter the pool.
        assert!(
            query
                .sql
                .contains("min(event_value) OVER (PARTITION BY dim_0_value, dim_0_label)")
        );
        assert!(
            query
                .sql
                .contains("max(event_value) OVER (PARTITION BY dim_0_value, dim_0_label)")
        );
        assert!(
            query
                .sql
                .contains("GROUP BY dim_0_value, dim_0_label, bin_idx")
        );
        assert!(!query.sql.contains("PARTITION BY entity_id"));
        assert!(query.sql.contains("JOIN identity.account_assignment"));
        assert!(query.sql.contains("JOIN identity.person_map"));
        // Same deterministic fixed-width arithmetic as the per-entity shape.
        assert!(query.sql.contains("least(9,"));
        assert!(query.sql.contains("* 10 /"));
        assert!(!query.sql.contains("histogram("));
        // Unbounded group cardinality rides the shared row limit.
        assert!(query.sql.contains(&format!("LIMIT {}", query_row_limit())));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn transform_wraps_every_query_shape() {
        let mut def = ratio_metric();
        def.transform = Some(ValueTransform {
            clamp_max: Some(100.0),
            ..ValueTransform::default()
        });
        let ts = compile_timeseries_query(&def, &request(), QueryBucket::Day, &[], &[], None);
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
        // The pooled shape bins the same transformed alias.
        let pooled =
            compile_pooled_histogram_query(&median, &request(), &["source".to_owned()], &[]);
        assert!(
            pooled
                .sql
                .contains("if((value) IS NULL, NULL, least(100.0, value))")
                && pooled.sql.contains("AS event_value"),
            "pooled histogram must transform into the event_value alias: {}",
            pooled.sql
        );
        assert_eq!(pooled.sql.matches('?').count(), pooled.params.len());
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

    fn flagged_input(
        role: MetricInputRole,
        measure_key: &str,
        collapse: AliasCollapse,
    ) -> MetricInput {
        MetricInput {
            alias_collapse: collapse,
            ..input(role, measure_key)
        }
    }

    fn flag_sum_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(vec!["tool"]),
            spec: ComputationSpec::Sum {
                value: flagged_input(MetricInputRole::Value, "active_day", AliasCollapse::Max),
            },
        }
    }

    fn inverse_flag_sum_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(vec!["tool"]),
            spec: ComputationSpec::Sum {
                value: flagged_input(
                    MetricInputRole::Value,
                    "meeting_free_day",
                    AliasCollapse::Min,
                ),
            },
        }
    }

    fn flag_ratio_metric() -> MetricDefinition {
        MetricDefinition {
            transform: None,
            base: base(vec!["tool"]),
            spec: ComputationSpec::Ratio {
                numerator: input(MetricInputRole::Numerator, "accepted_lines"),
                denominator: flagged_input(
                    MetricInputRole::Denominator,
                    "active_day",
                    AliasCollapse::Max,
                ),
                scale: 1.0,
                denominator_aggregation: RatioDenominatorAggregation::Sum,
            },
        }
    }

    #[test]
    fn a_person_read_resolves_through_the_live_map_and_groups_by_the_person() {
        let query = compile_period_batch_query(&[&sum_metric()], &request(), &[]);

        assert!(
            query
                .sql
                .contains("LEFT JOIN identity.account_assignment AS account_map")
        );
        assert!(
            query
                .sql
                .contains("LEFT JOIN identity.person_map AS person_map")
        );
        assert!(query.sql.contains("ON person_map.email = obs.entity_id"));
        assert!(
            query.sql.contains(
                "(obs.entity_id IN (SELECT email FROM identity.person_map WHERE person_id IN (?, ?)) OR coalesce(obs.account_id, '') != '')"
            ),
            "the prune keeps the requested people's email parts plus every account-carrying row"
        );
        assert!(
            query.sql.contains("WHERE resolved_person_id IN (?, ?)"),
            "the resolved filter is exact, whatever the prune let through"
        );
        assert!(query.sql.contains("resolved_person_id AS entity_id"));
        assert!(query.sql.contains("GROUP BY entity_id"));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn an_account_binding_wins_over_the_email_and_an_excluded_one_terminates() {
        let query = compile_period_batch_query(&[&sum_metric()], &request(), &[]);

        assert!(
            query
                .sql
                .contains("coalesce(account_map.account_id, '') != ''"),
            "the account arm is decided before the email arm"
        );
        assert!(
            query
                .sql
                .contains("= toUUID('ffffffff-ffff-ffff-ffff-ffffffffffff')"),
            "an account bound to the excluded person resolves to nobody"
        );
        assert!(
            query
                .sql
                .contains("sipHash128(coalesce(obs.account_source_id, ''))"),
            "the binding key uses identity's minted source id form"
        );
        assert!(
            query
                .sql
                .contains("account_map.account_id = lower(trimBoth(coalesce(obs.account_id, '')))"),
            "the fact side meets the map's lowered account id"
        );
    }

    #[test]
    fn a_custom_source_is_never_resolved_at_query_time() {
        let query = compile_period_batch_query(&[&custom_sum_metric()], &request(), &[]);

        assert!(!query.sql.contains("person_map"));
        assert!(query.sql.contains("AND entity_id IN (?, ?)"));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn a_flag_measure_collapses_per_person_day_before_the_outer_sum() {
        let query = compile_period_batch_query(&[&flag_sum_metric()], &request(), &[]);

        assert!(query.sql.contains("max(value)"), "flag collapses with max");
        assert!(
            query.sql.contains(
                "GROUP BY tenant_id, entity_type, source_key, measure_key, metric_date, entity_id, dimensions, subject_key"
            ),
            "collapse groups at the relation's own grain with the person in place of the identity"
        );
        assert!(
            query.sql.contains("sumIfOrNull(value"),
            "the outer aggregate still sums the collapsed days"
        );
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn an_inverse_flag_measure_collapses_with_min() {
        let query = compile_period_batch_query(&[&inverse_flag_sum_metric()], &request(), &[]);

        assert!(query.sql.contains("min(value)"));
        assert!(!query.sql.contains("max(value)"));
    }

    #[test]
    fn only_the_flagged_half_of_a_ratio_collapses() {
        let query = compile_period_batch_query(&[&flag_ratio_metric()], &request(), &[]);

        assert!(
            query
                .sql
                .contains("(source_key, measure_key) = ('ai_usage', 'active_day'), max(value)")
        );
        assert!(
            query.sql.contains("sum(value))"),
            "the multiIf falls through to sum for every unflagged measure"
        );
    }

    #[test]
    fn an_unflagged_read_has_no_collapse_stage() {
        for def in [sum_metric(), median_metric(), distinct_count_metric()] {
            let query = compile_period_batch_query(&[&def], &request(), &[]);
            assert!(
                !query.sql.contains("WITH resolved_rows AS"),
                "{} should not collapse",
                def.key()
            );
        }
    }

    #[test]
    fn a_peer_read_resolves_the_whole_cohort_not_only_the_targets() {
        let query = compile_peer_batch_query(
            &[&sum_metric()],
            &request(),
            "org_unit",
            PeerPopulation::DeclaredCohort,
            &[],
        );

        assert!(
            query
                .sql
                .contains("WHERE toString(person_id) IN (SELECT entity_id FROM cohort)"),
            "the map is scoped to cohort membership"
        );
        assert!(
            query.sql.contains("AND entity_id IN (?, ?)"),
            "targets still filter person ids against the person-grain cohort relation"
        );
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }

    #[test]
    fn a_flagged_neighbor_never_collapses_a_medians_event_rows() {
        let query =
            compile_period_batch_query(&[&flag_sum_metric(), &median_metric()], &request(), &[]);

        assert!(
            query
                .sql
                .contains("WHERE (source_key, measure_key) IN (('ai_usage', 'active_day'))"),
            "only the flagged pairs enter the grouped branch"
        );
        assert!(
            query
                .sql
                .contains("WHERE (source_key, measure_key) NOT IN (('ai_usage', 'active_day'))"),
            "every other measure's rows pass through ungrouped"
        );
        assert!(query.sql.contains("UNION ALL"));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
    }
}
