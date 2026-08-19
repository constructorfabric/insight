//! Custom-metric authoring model — the `origin='custom'` counterpart to the
//! builtin `registry.yaml` reconcile.
//!
//! A custom metric is a tenant-scoped metric graph (definition + one custom
//! observation source carrying inline SQL + its measures/dimensions + the
//! role→measure inputs) authored at runtime through `/v1/metrics*`. The graph
//! is validated as pure data here; the repository writes it as raw SQL in one
//! transaction, always `origin='custom'` and scoped to the session tenant.
//! Builtins are invisible to every query in this module, so the API can never
//! mutate a `registry.yaml`-owned row.

use std::collections::BTreeMap;

use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbErr, FromQueryResult, Statement, TransactionTrait, Value,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::metric_definitions::definition::{
    MetricComputation, MetricDirection, MetricFormat, MetricInputRole, ValueTransform,
};
use crate::domain::metric_key::parse_metric_key;
use crate::domain::query_gate::validate_custom_observation_sql;

#[cfg(test)]
#[path = "metric_crud_live_tests.rs"]
mod live_tests;

/// Upper bounds on an authored graph — every unbounded input is capped at the
/// edge so one request cannot fan out into an unbounded write.
const MAX_MEASURES: usize = 64;
const MAX_DIMENSIONS: usize = 64;
const MAX_TAGS: usize = 64;
const MAX_INPUTS: usize = 2;
const MAX_OBSERVATION_SQL_BYTES: usize = 64 * 1024;

/// Maximum metrics accepted by a single import request.
pub const MAX_IMPORT_METRICS: usize = 500;

/// Upper bound on how many custom metrics `GET /v1/metrics` and
/// `GET /v1/metrics/export` return, so neither the response nor the export's
/// per-metric fetch fan-out is unbounded.
const MAX_LIST_METRICS: usize = 1000;

// ── DTOs ─────────────────────────────────────────────────────

/// A portable custom-metric graph — the create/update body, the export item,
/// and the get/list detail shape. `origin` is response-only: always `"custom"`
/// on output, omitted from exports, and ignored on input (writes force
/// `custom`).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CustomMetric {
    pub metric_key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_label: Option<String>,
    /// The single topic this metric groups under within its family; a
    /// lowercase snake-case slug. Optional for custom metrics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    pub entity_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    pub format: MetricFormat,
    pub direction: MetricDirection,
    pub computation: MetricComputation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_cohort_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<ValueTransform>,
    pub source_key: String,
    pub observation_sql: String,
    pub measures: Vec<String>,
    pub dimensions: Vec<String>,
    /// Cross-cutting filter labels; lowercase snake-case slugs, unique per
    /// metric. Optional — defaults to empty.
    #[serde(default)]
    pub tags: Vec<String>,
    pub inputs: Vec<CustomMetricInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// One role→measure binding of a custom metric.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CustomMetricInput {
    pub role: MetricInputRole,
    pub measure_key: String,
}

/// List item — display fields only, no SQL body.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CustomMetricSummary {
    pub metric_key: String,
    pub label: String,
    /// Grouping subject, so the management list can partition custom metrics
    /// by topic like the definitions listing; absent when none is declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub computation: MetricComputation,
    pub entity_type: String,
}

/// `GET /v1/metrics` envelope.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CustomMetricListResponse {
    pub items: Vec<CustomMetricSummary>,
}

/// `GET /v1/metrics/export` envelope — the tenant's custom metric graphs.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ExportCustomMetricsResponse {
    pub metrics: Vec<CustomMetric>,
}

/// `POST /v1/metrics/import` body.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ImportCustomMetricsRequest {
    pub metrics: Vec<CustomMetric>,
}

/// `POST /v1/metrics/import` result — counts landed and the `metric_key`s
/// skipped because they already existed for the tenant.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ImportCustomMetricsResponse {
    pub imported: usize,
    pub skipped: Vec<String>,
}

impl toolkit::api::api_dto::RequestApiDto for CustomMetric {}
impl toolkit::api::api_dto::ResponseApiDto for CustomMetric {}
impl toolkit::api::api_dto::ResponseApiDto for CustomMetricListResponse {}
impl toolkit::api::api_dto::ResponseApiDto for ExportCustomMetricsResponse {}
impl toolkit::api::api_dto::RequestApiDto for ImportCustomMetricsRequest {}
impl toolkit::api::api_dto::ResponseApiDto for ImportCustomMetricsResponse {}

// ── Validation (pure) ────────────────────────────────────────

/// A rejected field of an authored graph: the wire field name and a short,
/// user-facing reason. Mapped to a 400 `field_violation` by the handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphViolation {
    pub field: &'static str,
    pub reason: String,
}

/// Strip trailing whitespace and statement terminators from observation SQL.
/// The single-SELECT gate accepts a trailing `;`, but the compiler wraps the
/// SQL as `FROM (<sql>)`, where a trailing `;` is a syntax error — so it is
/// removed once, at the write boundary, for both the probe and every render.
pub fn normalize_observation_sql(sql: &str) -> String {
    sql.trim().trim_end_matches(';').trim_end().to_owned()
}

fn violation(field: &'static str, reason: impl Into<String>) -> GraphViolation {
    GraphViolation {
        field,
        reason: reason.into(),
    }
}

/// A source/measure/dimension key: `^[a-z][a-z0-9_]*$`.
fn is_simple_key(value: &str) -> bool {
    let mut chars = value.chars();
    let leads = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    leads && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Validate an authored graph as pure data. The first violation wins so the
/// caller gets one precise field to fix.
///
/// # Errors
///
/// Returns a [`GraphViolation`] for a malformed key, an out-of-bounds list, a
/// computation/scale/inputs mismatch, or observation SQL that is not a single
/// read statement.
pub fn validate_graph(graph: &CustomMetric) -> Result<(), GraphViolation> {
    parse_metric_key(&graph.metric_key).map_err(|e| violation("metric_key", e.to_string()))?;

    if !is_simple_key(&graph.source_key) {
        return Err(violation("source_key", "must match ^[a-z][a-z0-9_]*$"));
    }
    if !is_simple_key(&graph.entity_type) {
        return Err(violation("entity_type", "must match ^[a-z][a-z0-9_]*$"));
    }
    if graph
        .peer_cohort_key
        .as_deref()
        .is_some_and(|cohort| !is_simple_key(cohort))
    {
        return Err(violation("peer_cohort_key", "must match ^[a-z][a-z0-9_]*$"));
    }
    if graph
        .subject
        .as_deref()
        .is_some_and(|subject| !is_simple_key(subject))
    {
        return Err(violation("subject", "must match ^[a-z][a-z0-9_]*$"));
    }

    validate_measures(graph)?;
    validate_dimensions(graph)?;
    validate_tags(graph)?;
    validate_observation_sql(graph)?;
    validate_inputs(graph)?;

    Ok(())
}

fn validate_tags(graph: &CustomMetric) -> Result<(), GraphViolation> {
    if graph.tags.len() > MAX_TAGS {
        return Err(violation(
            "tags",
            format!("at most {MAX_TAGS} tags are allowed"),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for tag in &graph.tags {
        if !is_simple_key(tag) {
            return Err(violation("tags", "must match ^[a-z][a-z0-9_]*$"));
        }
        if !seen.insert(tag.as_str()) {
            return Err(violation("tags", "tag keys must be unique"));
        }
    }
    Ok(())
}

fn validate_measures(graph: &CustomMetric) -> Result<(), GraphViolation> {
    if graph.measures.is_empty() {
        return Err(violation("measures", "at least one measure is required"));
    }
    if graph.measures.len() > MAX_MEASURES {
        return Err(violation(
            "measures",
            format!("at most {MAX_MEASURES} measures are allowed"),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for measure in &graph.measures {
        if !is_simple_key(measure) {
            return Err(violation("measures", "must match ^[a-z][a-z0-9_]*$"));
        }
        if !seen.insert(measure.as_str()) {
            return Err(violation("measures", "measure keys must be unique"));
        }
    }
    Ok(())
}

fn validate_dimensions(graph: &CustomMetric) -> Result<(), GraphViolation> {
    if graph.dimensions.len() > MAX_DIMENSIONS {
        return Err(violation(
            "dimensions",
            format!("at most {MAX_DIMENSIONS} dimensions are allowed"),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for dimension in &graph.dimensions {
        if !is_simple_key(dimension) {
            return Err(violation("dimensions", "must match ^[a-z][a-z0-9_]*$"));
        }
        if !seen.insert(dimension.as_str()) {
            return Err(violation("dimensions", "dimension keys must be unique"));
        }
    }
    Ok(())
}

fn validate_observation_sql(graph: &CustomMetric) -> Result<(), GraphViolation> {
    if graph.observation_sql.len() > MAX_OBSERVATION_SQL_BYTES {
        return Err(violation(
            "observation_sql",
            format!("must be at most {MAX_OBSERVATION_SQL_BYTES} bytes"),
        ));
    }
    validate_custom_observation_sql(&graph.observation_sql)
        .map_err(|reason| violation("observation_sql", reason))
}

fn validate_inputs(graph: &CustomMetric) -> Result<(), GraphViolation> {
    if graph.inputs.is_empty() || graph.inputs.len() > MAX_INPUTS {
        return Err(violation(
            "inputs",
            "one input for sum/median/distinct_count, two for ratio",
        ));
    }
    for input in &graph.inputs {
        if !graph.measures.iter().any(|m| m == &input.measure_key) {
            return Err(violation(
                "inputs",
                format!("input measure `{}` is not in measures", input.measure_key),
            ));
        }
    }

    let roles: Vec<MetricInputRole> = graph.inputs.iter().map(|input| input.role).collect();
    match graph.computation {
        MetricComputation::Sum | MetricComputation::Median | MetricComputation::DistinctCount => {
            if roles != [MetricInputRole::Value] {
                return Err(violation(
                    "inputs",
                    "sum/median/distinct_count take exactly one value input",
                ));
            }
            if graph.scale.is_some() {
                return Err(violation("scale", "only ratio metrics carry a scale"));
            }
        }
        MetricComputation::Ratio => {
            let has_numerator = roles.contains(&MetricInputRole::Numerator);
            let has_denominator = roles.contains(&MetricInputRole::Denominator);
            if roles.len() != 2 || !has_numerator || !has_denominator {
                return Err(violation(
                    "inputs",
                    "ratio takes one numerator and one denominator input",
                ));
            }
            if graph.scale.is_none() {
                return Err(violation("scale", "ratio metrics require a scale"));
            }
        }
    }
    Ok(())
}

// ── Repository (I/O) ─────────────────────────────────────────

/// Outcome of a create/import attempt for one metric.
#[derive(Debug)]
pub enum WriteOutcome {
    Created,
    AlreadyExists,
}

/// Insert a validated graph as an `origin='custom'` metric for `tenant_id` in
/// one transaction. Returns [`WriteOutcome::AlreadyExists`] (no write) when the
/// tenant already has that `metric_key` or `source_key`.
///
/// # Errors
///
/// Returns [`DbErr`] on any database failure other than a uniqueness conflict.
pub async fn create_custom_metric(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    graph: &CustomMetric,
) -> Result<WriteOutcome, DbErr> {
    let txn = db.begin().await?;
    // The existence checks run inside the transaction that inserts, so two
    // concurrent creates of the same key cannot both pass them.
    if metric_exists(&txn, tenant_id, &graph.metric_key).await?
        || source_exists(&txn, tenant_id, &graph.source_key).await?
    {
        txn.rollback().await?;
        return Ok(WriteOutcome::AlreadyExists);
    }

    insert_graph(&txn, tenant_id, graph).await?;
    txn.commit().await?;

    Ok(WriteOutcome::Created)
}

/// The result of a replace: the metric was replaced, no custom metric existed
/// under the key, or the new graph's `source_key` already belongs to another of
/// the tenant's custom metrics.
#[derive(Debug, PartialEq, Eq)]
pub enum ReplaceOutcome {
    Replaced,
    NotFound,
    SourceConflict,
}

/// Replace an existing custom metric with a validated graph, in one
/// transaction. The path `metric_key` is authoritative; the body's is
/// overwritten with it before this call.
///
/// # Errors
///
/// Returns [`DbErr`] on any database failure.
pub async fn replace_custom_metric(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    metric_key: &str,
    graph: &CustomMetric,
) -> Result<ReplaceOutcome, DbErr> {
    let txn = db.begin().await?;
    let deleted = delete_graph(&txn, tenant_id, metric_key).await?;
    if !deleted {
        txn.rollback().await?;
        return Ok(ReplaceOutcome::NotFound);
    }
    // The old source is gone with the old graph, so a reused source_key is fine;
    // a source_key belonging to a *different* custom metric is a conflict, the
    // same invariant create enforces.
    if source_exists(&txn, tenant_id, &graph.source_key).await? {
        txn.rollback().await?;
        return Ok(ReplaceOutcome::SourceConflict);
    }
    insert_graph(&txn, tenant_id, graph).await?;
    txn.commit().await?;
    Ok(ReplaceOutcome::Replaced)
}

/// Import a validated batch as `origin='custom'` metrics for `tenant_id` in one
/// transaction — either every new graph lands or none does. Returns the
/// `metric_key`s skipped because the tenant already owns them (or the same key
/// appears twice in the batch).
///
/// # Errors
///
/// Returns [`DbErr`] on any database failure; on error nothing is written.
pub async fn import_custom_metrics(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    graphs: &[CustomMetric],
) -> Result<Vec<String>, DbErr> {
    let txn = db.begin().await?;
    let mut skipped = Vec::new();
    for graph in graphs {
        if metric_exists(&txn, tenant_id, &graph.metric_key).await?
            || source_exists(&txn, tenant_id, &graph.source_key).await?
        {
            skipped.push(graph.metric_key.clone());
            continue;
        }
        insert_graph(&txn, tenant_id, graph).await?;
    }
    txn.commit().await?;
    Ok(skipped)
}

/// Delete a tenant's custom metric (definition + its custom source) by
/// `metric_key`. Returns `false` when none exists. Builtins never match.
///
/// # Errors
///
/// Returns [`DbErr`] on any database failure.
pub async fn delete_custom_metric(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    metric_key: &str,
) -> Result<bool, DbErr> {
    let txn = db.begin().await?;
    let deleted = delete_graph(&txn, tenant_id, metric_key).await?;
    txn.commit().await?;
    Ok(deleted)
}

// One INSERT per graph table (source, measures, source dimensions,
// definition, inputs, definition dimensions); keeping them in one function
// makes the write order and the shared generated ids explicit in one place.
#[expect(
    clippy::too_many_lines,
    reason = "one INSERT per graph table; splitting hides the write order"
)]
async fn insert_graph<C: ConnectionTrait>(
    conn: &C,
    tenant_id: Uuid,
    graph: &CustomMetric,
) -> Result<(), DbErr> {
    let source_id = Uuid::now_v7();
    let definition_id = Uuid::now_v7();

    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "INSERT INTO metric_sources \
            (id, tenant_id, source_key, source_kind, source_ref, observation_sql, origin, is_enabled, schema_status) \
         VALUES (?, ?, ?, 'custom_observation_sql', ?, ?, 'custom', TRUE, 'unchecked')",
        [
            uuid_value(source_id),
            uuid_value(tenant_id),
            Value::from(graph.source_key.as_str()),
            // source_ref is the managed relation name for a managed source; a
            // custom source reads from its observation_sql, so its source_ref
            // is the (unused-at-read) source key, satisfying the NOT NULL column.
            Value::from(graph.source_key.as_str()),
            Value::from(graph.observation_sql.as_str()),
        ],
    ))
    .await?;

    let mut measure_ids: BTreeMap<&str, Uuid> = BTreeMap::new();
    for measure in &graph.measures {
        let measure_id = Uuid::now_v7();
        measure_ids.insert(measure.as_str(), measure_id);
        conn.execute(Statement::from_sql_and_values(
            conn.get_database_backend(),
            "INSERT INTO metric_source_measures (id, source_id, measure_key, is_enabled) \
             VALUES (?, ?, ?, TRUE)",
            [
                uuid_value(measure_id),
                uuid_value(source_id),
                Value::from(measure.as_str()),
            ],
        ))
        .await?;
    }

    let mut dimension_ids: BTreeMap<&str, Uuid> = BTreeMap::new();
    for (order, dimension) in graph.dimensions.iter().enumerate() {
        let dimension_id = Uuid::now_v7();
        dimension_ids.insert(dimension.as_str(), dimension_id);
        conn.execute(Statement::from_sql_and_values(
            conn.get_database_backend(),
            "INSERT INTO metric_source_dimensions (id, source_id, dimension_key, display_order) \
             VALUES (?, ?, ?, ?)",
            [
                uuid_value(dimension_id),
                uuid_value(source_id),
                Value::from(dimension.as_str()),
                Value::from(order_value(order)),
            ],
        ))
        .await?;
    }

    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "INSERT INTO metric_definitions \
            (id, tenant_id, metric_key, label, short_label, subject, description, explanation, unit, \
             format, direction, entity_type, computation_type, scale, transform_multiplier, \
             transform_offset, transform_clamp_min, transform_clamp_max, peer_cohort_key, \
             origin, is_enabled, schema_status) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'custom', TRUE, 'unchecked')",
        [
            uuid_value(definition_id),
            uuid_value(tenant_id),
            Value::from(graph.metric_key.as_str()),
            Value::from(graph.label.as_str()),
            nullable_str(graph.short_label.as_deref()),
            nullable_str(graph.subject.as_deref()),
            nullable_str(graph.description.as_deref()),
            nullable_str(graph.explanation.as_deref()),
            nullable_str(graph.unit.as_deref()),
            Value::from(graph.format.as_db()),
            Value::from(graph.direction.as_db()),
            Value::from(graph.entity_type.as_str()),
            Value::from(graph.computation.as_db()),
            nullable_f64(graph.scale),
            nullable_f64(graph.transform.and_then(|t| t.multiplier)),
            nullable_f64(graph.transform.and_then(|t| t.offset)),
            nullable_f64(graph.transform.and_then(|t| t.clamp_min)),
            nullable_f64(graph.transform.and_then(|t| t.clamp_max)),
            nullable_str(graph.peer_cohort_key.as_deref()),
        ],
    ))
    .await?;

    for input in &graph.inputs {
        let measure_id = measure_ids.get(input.measure_key.as_str()).ok_or_else(|| {
            DbErr::Custom(format!(
                "input measure {} missing from measures",
                input.measure_key
            ))
        })?;
        conn.execute(Statement::from_sql_and_values(
            conn.get_database_backend(),
            "INSERT INTO metric_definition_inputs \
                (id, metric_definition_id, input_role, source_measure_id) \
             VALUES (?, ?, ?, ?)",
            [
                uuid_value(Uuid::now_v7()),
                uuid_value(definition_id),
                Value::from(input.role.as_db()),
                uuid_value(*measure_id),
            ],
        ))
        .await?;
    }

    for (order, dimension) in graph.dimensions.iter().enumerate() {
        let dimension_id = dimension_ids.get(dimension.as_str()).ok_or_else(|| {
            DbErr::Custom(format!(
                "dimension {dimension} missing from source dimensions"
            ))
        })?;
        conn.execute(Statement::from_sql_and_values(
            conn.get_database_backend(),
            "INSERT INTO metric_definition_dimensions \
                (id, metric_definition_id, source_dimension_id, display_order) \
             VALUES (?, ?, ?, ?)",
            [
                uuid_value(Uuid::now_v7()),
                uuid_value(definition_id),
                uuid_value(*dimension_id),
                Value::from(order_value(order)),
            ],
        ))
        .await?;
    }

    for (order, tag) in graph.tags.iter().enumerate() {
        conn.execute(Statement::from_sql_and_values(
            conn.get_database_backend(),
            "INSERT INTO metric_definition_tags \
                (id, metric_definition_id, tag, display_order) \
             VALUES (?, ?, ?, ?)",
            [
                uuid_value(Uuid::now_v7()),
                uuid_value(definition_id),
                Value::from(tag.as_str()),
                Value::from(order_value(order)),
            ],
        ))
        .await?;
    }

    Ok(())
}

/// Delete a tenant's custom definition and its source. The definition is
/// resolved from `metric_definitions` directly (not through the inputs join),
/// so a definition whose input rows are gone is still deletable rather than a
/// listed-but-unremovable ghost. The source is resolved through the inputs
/// while they still exist and dropped best-effort afterward; the definition
/// delete cascades its inputs and dimension links first, so the source delete
/// then cascades the measures/source-dimensions no input still references.
async fn delete_graph<C: ConnectionTrait>(
    conn: &C,
    tenant_id: Uuid,
    metric_key: &str,
) -> Result<bool, DbErr> {
    let Some(definition) = fetch_definition_row(conn, tenant_id, metric_key).await? else {
        return Ok(false);
    };
    let definition_id = definition.definition_id;
    let source_id = fetch_source_row(conn, definition_id)
        .await?
        .map(|source| source.source_id);

    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "DELETE FROM metric_definitions WHERE id = ?",
        [uuid_value(definition_id)],
    ))
    .await?;
    if let Some(source_id) = source_id {
        conn.execute(Statement::from_sql_and_values(
            conn.get_database_backend(),
            "DELETE FROM metric_sources WHERE id = ? AND origin = 'custom'",
            [uuid_value(source_id)],
        ))
        .await?;
    }
    Ok(true)
}

/// The tenant's custom metric graphs, ordered by `metric_key`.
///
/// # Errors
///
/// Returns [`DbErr`] on any database failure.
pub async fn list_custom_metrics(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<CustomMetricSummary>, DbErr> {
    #[derive(FromQueryResult)]
    struct Row {
        metric_key: String,
        label: String,
        subject: Option<String>,
        computation_type: String,
        entity_type: String,
    }

    let rows = Row::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT metric_key, label, subject, computation_type, entity_type \
         FROM metric_definitions \
         WHERE origin = 'custom' AND tenant_id = ? \
         ORDER BY metric_key \
         LIMIT ?",
        [uuid_value(tenant_id), Value::from(MAX_LIST_METRICS as u64)],
    ))
    .all(db)
    .await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let computation = MetricComputation::from_db(&row.computation_type)
            .ok_or_else(|| corrupt_value("computation_type", &row.computation_type))?;
        items.push(CustomMetricSummary {
            metric_key: row.metric_key,
            label: row.label,
            subject: row.subject,
            computation,
            entity_type: row.entity_type,
        });
    }
    Ok(items)
}

/// A tenant's custom metric graph by `metric_key`, or `None` when absent
/// (including when the key is a builtin — builtins never match `origin='custom'`).
///
/// # Errors
///
/// Returns [`DbErr`] on any database failure.
pub async fn fetch_custom_metric(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    metric_key: &str,
) -> Result<Option<CustomMetric>, DbErr> {
    let Some(row) = fetch_definition_row(db, tenant_id, metric_key).await? else {
        return Ok(None);
    };
    let Some(source) = fetch_source_row(db, row.definition_id).await? else {
        return Ok(None);
    };
    let measures = fetch_measure_keys(db, source.source_id).await?;
    let dimensions = fetch_dimension_keys(db, row.definition_id).await?;
    let tags = fetch_tag_keys(db, row.definition_id).await?;
    let inputs = fetch_input_rows(db, row.definition_id).await?;

    Ok(Some(
        row.into_graph(source, measures, dimensions, tags, inputs)?,
    ))
}

/// The tenant's full custom metric graphs (for export).
///
/// # Errors
///
/// Returns [`DbErr`] on any database failure.
pub async fn export_custom_metrics(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<CustomMetric>, DbErr> {
    let summaries = list_custom_metrics(db, tenant_id).await?;
    let mut metrics = Vec::with_capacity(summaries.len());
    for summary in summaries {
        if let Some(mut graph) = fetch_custom_metric(db, tenant_id, &summary.metric_key).await? {
            // Exports are portable: no tenant, no origin.
            graph.origin = None;
            metrics.push(graph);
        }
    }
    Ok(metrics)
}

// ── Repository helpers ───────────────────────────────────────

#[derive(FromQueryResult)]
struct DefinitionRow {
    definition_id: Uuid,
    metric_key: String,
    label: String,
    short_label: Option<String>,
    subject: Option<String>,
    description: Option<String>,
    explanation: Option<String>,
    unit: Option<String>,
    format: String,
    direction: String,
    entity_type: String,
    computation_type: String,
    scale: Option<f64>,
    transform_multiplier: Option<f64>,
    transform_offset: Option<f64>,
    transform_clamp_min: Option<f64>,
    transform_clamp_max: Option<f64>,
    peer_cohort_key: Option<String>,
}

#[derive(FromQueryResult)]
struct SourceRow {
    source_id: Uuid,
    source_key: String,
    observation_sql: Option<String>,
}

impl DefinitionRow {
    /// Rebuild the portable graph from stored rows, failing loud on any value
    /// that does not round-trip. A corrupt row must not be silently reshaped
    /// into a different metric — export would otherwise carry the substitute to
    /// another stand.
    fn into_graph(
        self,
        source: SourceRow,
        measures: Vec<String>,
        dimensions: Vec<String>,
        tags: Vec<String>,
        inputs: Vec<CustomMetricInput>,
    ) -> Result<CustomMetric, DbErr> {
        let transform = ValueTransform {
            multiplier: self.transform_multiplier,
            offset: self.transform_offset,
            clamp_min: self.transform_clamp_min,
            clamp_max: self.transform_clamp_max,
        };

        let format = MetricFormat::from_db(&self.format)
            .ok_or_else(|| corrupt_value("format", &self.format))?;
        let direction = MetricDirection::from_db(&self.direction)
            .ok_or_else(|| corrupt_value("direction", &self.direction))?;
        let computation = MetricComputation::from_db(&self.computation_type)
            .ok_or_else(|| corrupt_value("computation_type", &self.computation_type))?;
        let observation_sql = source
            .observation_sql
            .ok_or_else(|| corrupt_value("observation_sql", "NULL"))?;

        Ok(CustomMetric {
            metric_key: self.metric_key,
            label: self.label,
            short_label: self.short_label,
            subject: self.subject,
            description: self.description,
            explanation: self.explanation,
            entity_type: self.entity_type,
            unit: self.unit,
            format,
            direction,
            computation,
            scale: self.scale,
            peer_cohort_key: self.peer_cohort_key,
            transform: (!transform.is_identity()).then_some(transform),
            source_key: source.source_key,
            observation_sql,
            measures,
            dimensions,
            tags,
            inputs,
            origin: Some("custom".to_owned()),
        })
    }
}

/// A stored custom-metric value that does not round-trip through its `from_db`
/// parser: a corrupt row, surfaced loud rather than reshaped.
fn corrupt_value(field: &str, value: &str) -> DbErr {
    DbErr::Custom(format!("corrupt custom metric row: {field} = {value:?}"))
}

async fn fetch_definition_row<C: ConnectionTrait>(
    conn: &C,
    tenant_id: Uuid,
    metric_key: &str,
) -> Result<Option<DefinitionRow>, DbErr> {
    DefinitionRow::find_by_statement(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "SELECT \
            id AS definition_id, metric_key, label, short_label, subject, description, explanation, unit, \
            format, direction, entity_type, computation_type, \
            CAST(scale AS DOUBLE) AS scale, \
            CAST(transform_multiplier AS DOUBLE) AS transform_multiplier, \
            CAST(transform_offset AS DOUBLE) AS transform_offset, \
            CAST(transform_clamp_min AS DOUBLE) AS transform_clamp_min, \
            CAST(transform_clamp_max AS DOUBLE) AS transform_clamp_max, \
            peer_cohort_key \
         FROM metric_definitions \
         WHERE origin = 'custom' AND tenant_id = ? AND metric_key = ?",
        [uuid_value(tenant_id), Value::from(metric_key)],
    ))
    .one(conn)
    .await
}

async fn fetch_source_row<C: ConnectionTrait>(
    conn: &C,
    definition_id: Uuid,
) -> Result<Option<SourceRow>, DbErr> {
    SourceRow::find_by_statement(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "SELECT s.id AS source_id, s.source_key AS source_key, s.observation_sql AS observation_sql \
         FROM metric_sources s \
         INNER JOIN metric_source_measures m ON m.source_id = s.id \
         INNER JOIN metric_definition_inputs i ON i.source_measure_id = m.id \
         WHERE i.metric_definition_id = ? \
         LIMIT 1",
        [uuid_value(definition_id)],
    ))
    .one(conn)
    .await
}

async fn fetch_measure_keys<C: ConnectionTrait>(
    conn: &C,
    source_id: Uuid,
) -> Result<Vec<String>, DbErr> {
    #[derive(FromQueryResult)]
    struct Row {
        measure_key: String,
    }
    Row::find_by_statement(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "SELECT measure_key FROM metric_source_measures WHERE source_id = ? ORDER BY measure_key",
        [uuid_value(source_id)],
    ))
    .all(conn)
    .await
    .map(|rows| rows.into_iter().map(|row| row.measure_key).collect())
}

async fn fetch_dimension_keys<C: ConnectionTrait>(
    conn: &C,
    definition_id: Uuid,
) -> Result<Vec<String>, DbErr> {
    #[derive(FromQueryResult)]
    struct Row {
        dimension_key: String,
    }
    Row::find_by_statement(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "SELECT sd.dimension_key AS dimension_key \
         FROM metric_definition_dimensions dd \
         INNER JOIN metric_source_dimensions sd ON sd.id = dd.source_dimension_id \
         WHERE dd.metric_definition_id = ? \
         ORDER BY dd.display_order, sd.dimension_key",
        [uuid_value(definition_id)],
    ))
    .all(conn)
    .await
    .map(|rows| rows.into_iter().map(|row| row.dimension_key).collect())
}

async fn fetch_tag_keys<C: ConnectionTrait>(
    conn: &C,
    definition_id: Uuid,
) -> Result<Vec<String>, DbErr> {
    #[derive(FromQueryResult)]
    struct Row {
        tag: String,
    }
    Row::find_by_statement(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "SELECT t.tag AS tag \
         FROM metric_definition_tags t \
         WHERE t.metric_definition_id = ? \
         ORDER BY t.display_order, t.tag",
        [uuid_value(definition_id)],
    ))
    .all(conn)
    .await
    .map(|rows| rows.into_iter().map(|row| row.tag).collect())
}

async fn fetch_input_rows<C: ConnectionTrait>(
    conn: &C,
    definition_id: Uuid,
) -> Result<Vec<CustomMetricInput>, DbErr> {
    #[derive(FromQueryResult)]
    struct Row {
        input_role: String,
        measure_key: String,
    }
    Row::find_by_statement(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "SELECT i.input_role AS input_role, m.measure_key AS measure_key \
         FROM metric_definition_inputs i \
         INNER JOIN metric_source_measures m ON m.id = i.source_measure_id \
         WHERE i.metric_definition_id = ? \
         ORDER BY i.input_role, m.measure_key",
        [uuid_value(definition_id)],
    ))
    .all(conn)
    .await?
    .into_iter()
    .map(|row| {
        let role = MetricInputRole::from_db(&row.input_role)
            .ok_or_else(|| corrupt_value("input_role", &row.input_role))?;
        Ok(CustomMetricInput {
            role,
            measure_key: row.measure_key,
        })
    })
    .collect()
}

async fn metric_exists<C: ConnectionTrait>(
    conn: &C,
    tenant_id: Uuid,
    metric_key: &str,
) -> Result<bool, DbErr> {
    exists(
        conn,
        "SELECT 1 AS one FROM metric_definitions \
         WHERE origin = 'custom' AND tenant_id = ? AND metric_key = ? LIMIT 1",
        vec![uuid_value(tenant_id), Value::from(metric_key)],
    )
    .await
}

async fn source_exists<C: ConnectionTrait>(
    conn: &C,
    tenant_id: Uuid,
    source_key: &str,
) -> Result<bool, DbErr> {
    exists(
        conn,
        "SELECT 1 AS one FROM metric_sources \
         WHERE origin = 'custom' AND tenant_id = ? AND source_key = ? LIMIT 1",
        vec![uuid_value(tenant_id), Value::from(source_key)],
    )
    .await
}

async fn exists<C: ConnectionTrait>(
    conn: &C,
    sql: &str,
    values: Vec<Value>,
) -> Result<bool, DbErr> {
    Ok(conn
        .query_one(Statement::from_sql_and_values(
            conn.get_database_backend(),
            sql,
            values,
        ))
        .await?
        .is_some())
}

fn uuid_value(id: Uuid) -> Value {
    Value::Bytes(Some(Box::new(id.as_bytes().to_vec())))
}

fn order_value(idx: usize) -> i32 {
    i32::try_from(idx).unwrap_or(i32::MAX)
}

fn nullable_str(value: Option<&str>) -> Value {
    match value {
        Some(value) => Value::from(value),
        None => Value::String(None),
    }
}

fn nullable_f64(value: Option<f64>) -> Value {
    match value {
        Some(value) => Value::from(value),
        None => Value::Double(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum_graph() -> CustomMetric {
        CustomMetric {
            metric_key: "custom.example".to_owned(),
            label: "Example".to_owned(),
            short_label: None,
            subject: Some("activity".to_owned()),
            description: None,
            explanation: None,
            entity_type: "person".to_owned(),
            unit: None,
            format: MetricFormat::Integer,
            direction: MetricDirection::HigherIsBetter,
            computation: MetricComputation::Sum,
            scale: None,
            peer_cohort_key: None,
            transform: None,
            source_key: "custom_example".to_owned(),
            observation_sql: "SELECT tenant_id, source_key, entity_type, entity_id, metric_date, \
                measure_key, observed_at, value, subject_key, dimensions FROM src"
                .to_owned(),
            measures: vec!["events".to_owned()],
            dimensions: vec!["tool".to_owned()],
            tags: vec!["example".to_owned()],
            inputs: vec![CustomMetricInput {
                role: MetricInputRole::Value,
                measure_key: "events".to_owned(),
            }],
            origin: None,
        }
    }

    fn ratio_graph() -> CustomMetric {
        let mut graph = sum_graph();
        graph.computation = MetricComputation::Ratio;
        graph.scale = Some(100.0);
        graph.measures = vec!["num".to_owned(), "den".to_owned()];
        graph.inputs = vec![
            CustomMetricInput {
                role: MetricInputRole::Numerator,
                measure_key: "num".to_owned(),
            },
            CustomMetricInput {
                role: MetricInputRole::Denominator,
                measure_key: "den".to_owned(),
            },
        ];
        graph
    }

    #[track_caller]
    fn rejected_field(graph: &CustomMetric) -> &'static str {
        match validate_graph(graph) {
            Ok(()) => panic!("expected a graph violation"),
            Err(violation) => violation.field,
        }
    }

    #[test]
    fn normalize_strips_trailing_terminator() {
        // A trailing `;` passes the single-SELECT gate but breaks the compiler's
        // FROM (<sql>) wrap; it must be gone before the SQL is stored.
        assert_eq!(normalize_observation_sql("SELECT 1;"), "SELECT 1");
        assert_eq!(normalize_observation_sql("  SELECT 1 ;  "), "SELECT 1");
        assert_eq!(normalize_observation_sql("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn valid_sum_and_ratio_graphs_pass() {
        assert!(validate_graph(&sum_graph()).is_ok());
        assert!(validate_graph(&ratio_graph()).is_ok());
    }

    #[test]
    fn rejects_bad_metric_key_and_source_key() {
        let mut graph = sum_graph();
        graph.metric_key = "NoDot".to_owned();
        assert_eq!(rejected_field(&graph), "metric_key");

        let mut graph = sum_graph();
        graph.source_key = "Bad-Key".to_owned();
        assert_eq!(rejected_field(&graph), "source_key");
    }

    #[test]
    fn rejects_malformed_subject_and_tags() {
        let mut graph = sum_graph();
        graph.subject = Some("Bad Subject".to_owned());
        assert_eq!(rejected_field(&graph), "subject");

        let mut graph = sum_graph();
        graph.tags = vec!["Bad-Tag".to_owned()];
        assert_eq!(rejected_field(&graph), "tags");

        let mut graph = sum_graph();
        graph.tags = vec!["dup".to_owned(), "dup".to_owned()];
        assert_eq!(rejected_field(&graph), "tags");
    }

    #[test]
    fn accepts_absent_subject_and_empty_tags() {
        let mut graph = sum_graph();
        graph.subject = None;
        graph.tags = vec![];
        assert!(validate_graph(&graph).is_ok());
    }

    #[test]
    fn rejects_non_single_select_observation_sql() {
        let mut graph = sum_graph();
        graph.observation_sql = "SELECT 1; DROP TABLE t".to_owned();
        assert_eq!(rejected_field(&graph), "observation_sql");
    }

    #[test]
    fn sum_rejects_scale_and_wrong_input_shape() {
        let mut graph = sum_graph();
        graph.scale = Some(100.0);
        assert_eq!(rejected_field(&graph), "scale");

        let mut graph = sum_graph();
        graph.inputs.push(CustomMetricInput {
            role: MetricInputRole::Numerator,
            measure_key: "events".to_owned(),
        });
        assert_eq!(rejected_field(&graph), "inputs");
    }

    #[test]
    fn ratio_requires_scale_and_both_roles() {
        let mut graph = ratio_graph();
        graph.scale = None;
        assert_eq!(rejected_field(&graph), "scale");

        let mut graph = ratio_graph();
        graph.inputs = vec![CustomMetricInput {
            role: MetricInputRole::Numerator,
            measure_key: "num".to_owned(),
        }];
        assert_eq!(rejected_field(&graph), "inputs");
    }

    #[test]
    fn input_measure_must_be_declared() {
        let mut graph = sum_graph();
        graph.inputs = vec![CustomMetricInput {
            role: MetricInputRole::Value,
            measure_key: "not_declared".to_owned(),
        }];
        assert_eq!(rejected_field(&graph), "inputs");
    }

    fn definition_row() -> DefinitionRow {
        DefinitionRow {
            definition_id: Uuid::now_v7(),
            metric_key: "custom.example".to_owned(),
            label: "Example".to_owned(),
            short_label: None,
            subject: Some("activity".to_owned()),
            description: None,
            explanation: None,
            unit: None,
            format: "integer".to_owned(),
            direction: "neutral".to_owned(),
            entity_type: "person".to_owned(),
            computation_type: "sum".to_owned(),
            scale: None,
            transform_multiplier: None,
            transform_offset: None,
            transform_clamp_min: None,
            transform_clamp_max: None,
            peer_cohort_key: None,
        }
    }

    fn source_row() -> SourceRow {
        SourceRow {
            source_id: Uuid::now_v7(),
            source_key: "custom_example".to_owned(),
            observation_sql: Some("SELECT 1".to_owned()),
        }
    }

    #[test]
    fn into_graph_rebuilds_a_valid_row_and_stamps_origin() {
        let graph = definition_row()
            .into_graph(
                source_row(),
                vec!["events".to_owned()],
                vec![],
                vec!["rate".to_owned()],
                vec![],
            )
            .unwrap_or_else(|error| panic!("valid row must rebuild: {error}"));
        assert_eq!(graph.origin.as_deref(), Some("custom"));
        assert_eq!(graph.observation_sql, "SELECT 1");
        assert_eq!(graph.measures, vec!["events".to_owned()]);
        assert_eq!(graph.subject.as_deref(), Some("activity"));
        assert_eq!(graph.tags, vec!["rate".to_owned()]);
    }

    #[test]
    fn into_graph_fails_loud_on_corrupt_stored_values() {
        for mutate in [
            (|row: &mut DefinitionRow| row.format = "bogus".to_owned()) as fn(&mut DefinitionRow),
            |row: &mut DefinitionRow| row.direction = "bogus".to_owned(),
            |row: &mut DefinitionRow| row.computation_type = "bogus".to_owned(),
        ] {
            let mut row = definition_row();
            mutate(&mut row);
            assert!(
                row.into_graph(source_row(), vec![], vec![], vec![], vec![])
                    .is_err(),
                "a corrupt enum must not be silently reshaped"
            );
        }

        let mut source = source_row();
        source.observation_sql = None;
        assert!(
            definition_row()
                .into_graph(source, vec![], vec![], vec![], vec![])
                .is_err(),
            "a NULL observation_sql on a custom source is corrupt"
        );
    }
}
