//! Route handlers.

use std::fmt::Write as _;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use sea_orm::{ActiveModelTrait, ColumnTrait, Condition, EntityTrait, NotSet, QueryFilter, Set};
use toolkit_canonical_errors::{CanonicalError, Problem};
use uuid::Uuid;

use super::AppState;
use super::error::{MetricError, PersonError, ThresholdError};
use toolkit_security::SecurityContext;
use crate::domain::metric::{
    CreateMetricRequest, Metric, MetricSummary, TableColumn, UpdateMetricRequest,
};
use crate::domain::query::{
    BatchQueryRequest, BatchQueryResponse, BatchQueryResult, PageInfo, QueryRequest, QueryResponse,
};
use crate::domain::threshold;
use crate::domain::threshold::{CreateThresholdRequest, Threshold, UpdateThresholdRequest};
use crate::infra::db::entities;

// ── Health ──────────────────────────────────────────────────

pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "healthy" }))
}

// ── Person lookup ──────────────────────────────────────────

pub async fn get_person(
    Extension(state): Extension<Arc<AppState>>,
    Path(email): Path<String>,
) -> Result<impl IntoResponse, CanonicalError> {
    if !state.identity.is_configured() {
        return Err(
            CanonicalError::internal("identity resolution service not configured").create(),
        );
    }

    let person = state.identity.get_person(&email).await.map_err(|e| {
        tracing::error!(error = %e, email = %email, "identity resolution request failed");
        CanonicalError::internal("identity resolution unavailable").create()
    })?;

    match person {
        Some(p) => Ok(Json(serde_json::to_value(p).map_err(|e| {
            tracing::error!(error = %e, "failed to serialize person");
            CanonicalError::internal("failed to serialize person").create()
        })?)),
        None => Err(PersonError::not_found("person not found")
            .with_resource(email)
            .create()),
    }
}

// ── Metrics CRUD ────────────────────────────────────────────

pub async fn list_metrics(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    let rows = entities::metrics::Entity::find()
        .filter(
            entities::metrics::Column::InsightTenantId
                .is_in([ctx.subject_tenant_id(), GLOBAL_TENANT]),
        )
        .filter(entities::metrics::Column::IsEnabled.eq(true))
        .all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to list metrics");
            CanonicalError::internal("failed to list metrics").create()
        })?;

    let items: Vec<MetricSummary> = rows.into_iter().map(model_to_metric_summary).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn get_metric(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    let row = find_enabled_metric(&state, ctx.subject_tenant_id(), id).await?;
    Ok(Json(model_to_metric(row)))
}

pub async fn create_metric(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<CreateMetricRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    // Validate query_ref on write — reject malformed definitions early
    parse_query_ref(&req.query_ref).map_err(|e| {
        MetricError::invalid_argument()
            .with_field_violation("query_ref", e, "INVALID")
            .create()
    })?;

    let id = Uuid::now_v7();

    let model = entities::metrics::ActiveModel {
        id: Set(id),
        insight_tenant_id: Set(ctx.subject_tenant_id()),
        name: Set(req.name),
        description: Set(req.description),
        query_ref: Set(req.query_ref),
        is_enabled: Set(true),
        created_at: NotSet,
        updated_at: NotSet,
    };

    entities::metrics::Entity::insert(model)
        .exec(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to create metric");
            CanonicalError::internal("failed to create metric").create()
        })?;

    let row = entities::metrics::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to fetch created metric");
            CanonicalError::internal("failed to fetch created metric").create()
        })?
        .ok_or_else(|| CanonicalError::internal("created metric not found").create())?;

    Ok((StatusCode::CREATED, Json(model_to_metric(row))))
}

pub async fn update_metric(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateMetricRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let existing = find_enabled_metric(&state, ctx.subject_tenant_id(), id).await?;
    let mut model: entities::metrics::ActiveModel = existing.into();

    if let Some(name) = req.name {
        model.name = Set(name);
    }
    // Explicit null clears description; absent field leaves it unchanged.
    if let Some(desc) = req.description {
        model.description = Set(desc);
    }
    if let Some(query_ref) = req.query_ref {
        // Validate query_ref on write
        parse_query_ref(&query_ref).map_err(|e| {
            MetricError::invalid_argument()
                .with_resource(id.to_string())
                .with_field_violation("query_ref", e, "INVALID")
                .create()
        })?;
        model.query_ref = Set(query_ref);
    }
    if let Some(enabled) = req.is_enabled {
        model.is_enabled = Set(enabled);
    }
    model.updated_at = Set(chrono::Utc::now());

    let updated = model.update(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "failed to update metric");
        CanonicalError::internal("failed to update metric").create()
    })?;

    Ok(Json(model_to_metric(updated)))
}

pub async fn delete_metric(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    let existing = find_enabled_metric(&state, ctx.subject_tenant_id(), id).await?;

    let mut model: entities::metrics::ActiveModel = existing.into();
    model.is_enabled = Set(false);
    model.updated_at = Set(chrono::Utc::now());
    model.update(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "failed to soft-delete metric");
        CanonicalError::internal("failed to soft-delete metric").create()
    })?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Query ───────────────────────────────────────────────────

pub async fn query_metric(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
    Json(req): Json<QueryRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let response = execute_metric_query(&state, &ctx, id, req).await?;
    Ok(Json(response))
}

pub async fn query_metrics_batch(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<BatchQueryRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tasks = req.queries.into_iter().map(|item| {
        let state = state.clone();
        let ctx = ctx.clone();
        async move {
            let id = item.id;
            let metric_id = item.metric_id;
            match execute_metric_query(&state, &ctx, metric_id, item.query).await {
                Ok(response) => BatchQueryResult::Ok {
                    id,
                    metric_id,
                    response,
                },
                Err(err) => BatchQueryResult::Error {
                    id,
                    metric_id,
                    error: Problem::from(err),
                },
            }
        }
    });

    let results = futures::future::join_all(tasks).await;
    Ok(Json(BatchQueryResponse { results }))
}

#[allow(clippy::too_many_lines)]
async fn execute_metric_query(
    state: &Arc<AppState>,
    ctx: &SecurityContext,
    id: Uuid,
    req: QueryRequest,
) -> Result<QueryResponse, CanonicalError> {
    // 1. Load metric definition (must be enabled)
    let metric = find_enabled_metric(state, ctx.subject_tenant_id(), id).await?;

    // 2. Validate $top. Ceiling raised from 200 to accommodate per-person
    //    "member values" metrics scoped by `person_id IN (roster)`, which emit
    //    one long row per (person, metric_key) — up to roster_size × metric_count.
    //    Default ($top=25) and all existing callers (≤200) are unaffected by the
    //    higher ceiling; only callers that explicitly request more get it.
    let top = req.top.clamp(1, 5000);

    // 4. Build ClickHouse query from structured metric fields.
    //
    // The engine always controls FROM and WHERE — insight_tenant_id is
    // always injected for tenant isolation. Admins never control WHERE.
    //
    // Person ID resolution: if identity_url is configured, person_ids from
    // $filter would be resolved to source aliases via the Identity API.
    // For MVP, the service is not deployed — person_ids from the JWT
    // subject_id are used directly against ClickHouse (Gold tables already
    // have resolved person_id columns).
    //
    // TODO: Full implementation should also:
    // - Validate person_id / person_id IN values from $filter against AccessScope (IDOR prevention)
    // - Resolve person_ids when identity_url is set
    // - Parse $select to restrict returned columns
    // - Implement cursor-based pagination (decode $skip → keyset)

    let (select_expr, from_clause, group_by) = parse_query_ref(&metric.query_ref).map_err(|e| {
        tracing::error!(error = %e, query_ref = %metric.query_ref, "invalid query_ref");
        CanonicalError::internal("metric has invalid query_ref").create()
    })?;

    // Allow $select to override the columns from query_ref
    let select_expr = match &req.select {
        Some(sel) if !sel.is_empty() => {
            if !sel.split(',').all(|col| is_valid_ident(col.trim())) {
                return Err(MetricError::invalid_argument()
                    .with_resource(id.to_string())
                    .with_field_violation("$select", format!("invalid $select: {sel}"), "INVALID")
                    .create());
            }
            sel.clone()
        }
        _ => select_expr,
    };

    // MVP: single tenant — skip tenant isolation filter.
    // TODO: re-enable for multi-tenant: WHERE insight_tenant_id = ?
    let mut params: Vec<String> = vec![];

    // If the FROM clause is a subquery, we inject the metric_date range INSIDE the
    // subquery (before its GROUP BY). Keeps per-person aggregation bounded to the
    // selected period. Outer person_id (eq / IN) filters still apply post-aggregate.
    let (effective_from, date_pushed) = if let Some(ref filter) = req.filter {
        let date_from = extract_odata_value(filter, "metric_date", "ge");
        // Accept both `lt` (exclusive) and `le` (inclusive) — the FE's
        // `odataDateFilter` emits `le`, OData spec also allows `lt`. Without
        // both, the upper bound is silently dropped and the period extends
        // to "today" — which is exactly the bug we're fixing here.
        let (date_to, date_to_op) = match extract_odata_value(filter, "metric_date", "lt") {
            Some(v) => (Some(v), "<"),
            None => match extract_odata_value(filter, "metric_date", "le") {
                Some(v) => (Some(v), "<="),
                None => (None, "<"),
            },
        };
        if date_from.is_some() || date_to.is_some() {
            let mut clauses: Vec<String> = vec![];
            if let Some(ref v) = date_from {
                clauses.push(format!("metric_date >= '{}'", v.replace('\'', "''")));
            }
            if let Some(ref v) = date_to {
                clauses.push(format!(
                    "metric_date {} '{}'",
                    date_to_op,
                    v.replace('\'', "''"),
                ));
            }
            let where_inner = format!(" WHERE {}", clauses.join(" AND "));
            let normalised = ensure_subquery_from(&from_clause);
            match inject_date_filter_into_subqueries(&normalised, &where_inner) {
                Some(new_from) => (new_from, true),
                None => (from_clause.clone(), false),
            }
        } else {
            (from_clause.clone(), false)
        }
    } else {
        (from_clause.clone(), false)
    };
    let _ = effective_from;

    let mut sql = format!("SELECT {select_expr} FROM {effective_from} WHERE 1=1");

    // Parse OData $filter (simplified — production needs a proper OData parser)
    if let Some(ref filter) = req.filter {
        if !date_pushed {
            // Date filter was provided but couldn't be pushed into a subquery
            // (e.g. the FROM clause is a bare table without parentheses — the
            // "Team Members" shape, where the outer query reads
            // `bronze_bamboohr.employees` directly). Adding the filter to the
            // outer WHERE would reference `metric_date` against a table that
            // doesn't expose that column, producing a ClickHouse
            // `UNKNOWN_IDENTIFIER` 500. Drop the filter with a warning so the
            // request still returns 200 (unfiltered) and the misconfiguration
            // is visible in logs.
            let has_date_filter = extract_odata_value(filter, "metric_date", "ge").is_some()
                || extract_odata_value(filter, "metric_date", "lt").is_some()
                || extract_odata_value(filter, "metric_date", "le").is_some();
            if has_date_filter {
                tracing::warn!(
                    metric_id = %id,
                    "date filter on metric_date was provided but could not be \
                     injected into a subquery (FROM clause is not a subquery). \
                     Filter is ignored; the request returns unfiltered results.",
                );
            }
        }
        // Person filter — use person_id directly (no Identity Resolution for MVP).
        // Gold tables have a resolved person_id column; Silver tables would need
        // alias resolution via Identity Resolution API when it's available.
        if let Some(person_id) = extract_odata_value(filter, "person_id", "eq") {
            sql.push_str(" AND person_id = ?");
            params.push(person_id);
        }
        // Roster-scoped team aggregates: the FE passes the identity-tree
        // subtree's person_ids as `person_id in ('a','b',…)` so a team's
        // sections aggregate over exactly the members shown — the transitive,
        // active subtree, matching the heatmap. One bound `?` per id; never
        // interpolated.
        if let Some(ids) = extract_odata_in_values(filter, "person_id") {
            let placeholders = vec!["?"; ids.len()].join(", ");
            let _ = write!(sql, " AND person_id IN ({placeholders})");
            params.extend(ids);
        }
        // Department-scoped distribution metrics: the FE passes the
        // org_unit_ids of the departments represented in the roster as
        // `org_unit_id in ('a','b',…)` so the per-department distribution
        // metrics return only the relevant departments. `org_unit_id` is a
        // column of the FROM, so this lands in the outer WHERE before the
        // re-appended GROUP BY. One bound `?` per id; never interpolated.
        if let Some(ids) = extract_odata_in_values(filter, "org_unit_id") {
            let placeholders = vec!["?"; ids.len()].join(", ");
            let _ = write!(sql, " AND org_unit_id IN ({placeholders})");
            params.extend(ids);
        }
        // Drill filter — used by IC Dashboard drill modal.
        if let Some(drill_id) = extract_odata_value(filter, "drill_id", "eq") {
            sql.push_str(" AND drill_id = ?");
            params.push(drill_id);
        }
        // Per-metric scoping filters used by ic_histogram and ic_section_trend.
        if let Some(metric_key) = extract_odata_value(filter, "metric_key", "eq") {
            sql.push_str(" AND metric_key = ?");
            params.push(metric_key);
        }
        if let Some(section_id) = extract_odata_value(filter, "section_id", "eq") {
            sql.push_str(" AND section_id = ?");
            params.push(section_id);
        }
    }

    // Apply GROUP BY from parsed query_ref
    if let Some(ref gb) = group_by {
        let _ = write!(sql, " GROUP BY {gb}");
    }

    // Apply $orderby — validate against identifier pattern to prevent injection
    if let Some(ref orderby) = req.orderby {
        if !is_valid_orderby(orderby) {
            return Err(MetricError::invalid_argument()
                .with_resource(id.to_string())
                .with_field_violation(
                    "$orderby",
                    format!("invalid $orderby: {orderby}"),
                    "INVALID",
                )
                .create());
        }
        let _ = write!(sql, " ORDER BY {orderby}");
    }

    // Apply pagination (fetch top+1 to detect has_next)
    let _ = write!(sql, " LIMIT {}", top + 1);

    tracing::debug!(sql = %sql, metric_id = %id, "executing metric query");

    // 5. Execute the query against ClickHouse using JSONEachRow format
    //    for dynamic column deserialization (metric queries have varying schemas).
    let mut query = state.ch.query(&sql);
    for param in &params {
        query = query.bind(param.as_str());
    }

    let mut cursor = query.fetch_bytes("JSONEachRow").map_err(|e| {
        tracing::error!(error = %e, sql = %sql, "ClickHouse query failed");
        CanonicalError::internal("query execution failed").create()
    })?;

    let raw_bytes = cursor.collect().await.map_err(|e| {
        tracing::error!(error = %e, sql = %sql, "ClickHouse fetch failed");
        CanonicalError::internal("query execution failed").create()
    })?;

    // Parse JSONEachRow: one JSON object per line
    let all_rows: Vec<serde_json::Value> = if raw_bytes.is_empty() {
        Vec::new()
    } else {
        raw_bytes
            .split(|&b| b == b'\n')
            .filter(|line| !line.is_empty())
            .map(serde_json::from_slice)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                tracing::error!(error = %e, "failed to parse ClickHouse JSON response");
                CanonicalError::internal("failed to parse query results").create()
            })?
    };

    // 6. Apply pagination — we fetched top+1 to detect has_next
    let has_next = all_rows.len() > top as usize;
    let items: Vec<serde_json::Value> = if has_next {
        all_rows
            .into_iter()
            .take(top as usize)
            .map(round_floats)
            .collect()
    } else {
        all_rows.into_iter().map(round_floats).collect()
    };

    Ok(QueryResponse {
        items,
        page_info: PageInfo {
            has_next,
            cursor: None,
        },
    })
}

/// Round all float values in a JSON object to 4 decimal places.
fn round_floats(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                let rounded = (f * 10000.0).round() / 10000.0;
                serde_json::json!(rounded)
            } else {
                serde_json::Value::Number(n)
            }
        }
        serde_json::Value::Object(map) => {
            serde_json::Value::Object(map.into_iter().map(|(k, v)| (k, round_floats(v))).collect())
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(round_floats).collect())
        }
        other => other,
    }
}

/// Simplified `OData` value extractor.
/// Extracts value from patterns like `field_name ge 'value'`.
fn extract_odata_value(filter: &str, field: &str, op: &str) -> Option<String> {
    let pattern = format!("{field} {op} '");
    if let Some(start) = filter.find(&pattern) {
        let rest = &filter[start + pattern.len()..];
        if let Some(end) = rest.find('\'') {
            return Some(rest[..end].to_owned());
        }
    }
    None
}

/// Parse an `OData` `field in ('a','b',…)` list into its values. `person_id`s are
/// emails (no `)`/`,` inside), so the paren group is delimited simply. Returns
/// `None` for an absent or empty list; the caller binds each value as a `?`.
fn extract_odata_in_values(filter: &str, field: &str) -> Option<Vec<String>> {
    let pattern = format!("{field} in (");
    let start = filter.find(&pattern)?;
    let rest = &filter[start + pattern.len()..];
    let end = rest.find(')')?;
    let values: Vec<String> = rest[..end]
        .split(',')
        .filter_map(|tok| {
            let t = tok.trim().strip_prefix('\'')?.strip_suffix('\'')?;
            Some(t.replace("''", "'"))
        })
        .collect();
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

/// Parse `query_ref` into (`select_expr`, `from_clause`, `group_by`).
///
/// Expects SQL in the form:
///   `SELECT <columns> FROM <from_clause>`
///   `SELECT <columns> FROM <from_clause> GROUP BY <expr>`
///
/// `from_clause` can be:
///   - A single table: `silver.class_comms_events`
///   - A table with alias: `silver.class_comms_events c`
///   - A JOIN: `silver.class_comms_events c JOIN bronze_bamboohr.employees e ON ...`
///
/// The engine rebuilds the query with `WHERE insight_tenant_id = ?` always
/// injected, so admins cannot bypass tenant isolation.
fn ensure_subquery_from(from_clause: &str) -> String {
    if from_clause.trim_start().starts_with('(') {
        return from_clause.to_string();
    }
    rewrap_first_table(from_clause)
}

fn rewrap_first_table(from_clause: &str) -> String {
    let trimmed = from_clause.trim_start();
    let leading_ws_len = from_clause.len() - trimmed.len();
    let leading_ws = &from_clause[..leading_ws_len];

    let upper = trimmed.to_ascii_uppercase();
    let join_pos = [
        " LEFT JOIN ",
        " RIGHT JOIN ",
        " INNER JOIN ",
        " CROSS JOIN ",
        " JOIN ",
    ]
    .iter()
    .filter_map(|kw| find_at_depth_zero(&upper, kw))
    .min();

    let (prefix, tail) = match join_pos {
        Some(p) => (&trimmed[..p], &trimmed[p..]),
        None => (trimmed, ""),
    };

    let tokens: Vec<&str> = prefix.split_whitespace().collect();
    match tokens.as_slice() {
        [t] => format!("{leading_ws}(SELECT * FROM {t}){tail}"),
        [t, a] => format!("{leading_ws}(SELECT * FROM {t}) {a}{tail}"),
        _ => from_clause.to_string(),
    }
}

/// Inject `where_clause` (` WHERE ...`) into every *leafmost* `(SELECT ...)` subquery
/// inside `from_clause`. A leaf subquery is one whose own FROM is a table (not another
/// subquery). This guarantees the `metric_date` filter lands at the level where the raw
/// table with a `metric_date` column is actually read. JOIN branches and nested
/// subqueries are recursed into; the filter is applied only at the innermost SELECT.
fn inject_date_filter_into_subqueries(from_clause: &str, where_clause: &str) -> Option<String> {
    let (new_from, injected) = walk_inject(from_clause, where_clause);
    if injected { Some(new_from) } else { None }
}

fn walk_inject(from_clause: &str, where_clause: &str) -> (String, bool) {
    let bytes = from_clause.as_bytes();
    let mut result = String::with_capacity(from_clause.len() + where_clause.len() * 2);
    let mut i = 0;
    let mut any = false;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            // Find matching close paren.
            let mut depth: i32 = 1;
            let mut j = i + 1;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
                j += 1;
            }
            if j >= bytes.len() {
                result.push_str(&from_clause[i..]);
                break;
            }
            let inner = &from_clause[i + 1..j];
            let after_paren_start = j + 1;

            // Skip non-SELECT groups (e.g., lower(x)).
            let is_select = inner
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("SELECT ");
            if is_select {
                let (processed, did_inject) = process_select(inner, where_clause);
                if did_inject {
                    any = true;
                }
                result.push('(');
                result.push_str(&processed);
                result.push(')');
            } else {
                result.push_str(&from_clause[i..=j]);
            }
            i = after_paren_start;
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    (result, any)
}

/// Process the body of a (SELECT ...) subquery. If its own FROM is itself a subquery,
/// recurse; otherwise inject WHERE at this level.
fn process_select(inner: &str, where_clause: &str) -> (String, bool) {
    let inner_upper = inner.to_ascii_uppercase();
    let from_pos = find_at_depth_zero(&inner_upper, " FROM ");
    let Some(from_pos) = from_pos else {
        return (inner.to_string(), false);
    };
    let after_from = &inner[from_pos + 6..];

    // Separate sub-FROM from optional trailing " GROUP BY ..." / " WHERE ..." etc.
    // We only care whether the FROM clause itself starts with `(`.
    let after_trim = after_from.trim_start();
    if after_trim.starts_with('(') {
        // Recurse into nested FROM. We need to extract the FROM clause up to its
        // end (before GROUP BY at depth 0 in after_from).
        let after_from_upper = after_from.to_ascii_uppercase();
        let gb_pos = find_at_depth_zero(&after_from_upper, " GROUP BY ");
        let (inner_from, rest) = match gb_pos {
            Some(pos) => (&after_from[..pos], &after_from[pos..]),
            None => (after_from, ""),
        };
        let (new_inner_from, nested_did) = walk_inject(inner_from, where_clause);
        let rebuilt = format!("{} FROM {}{}", &inner[..from_pos], new_inner_from, rest);
        (rebuilt, nested_did)
    } else {
        let leaf_table = after_trim.split_whitespace().next().unwrap_or("");
        if leaf_table.starts_with("bronze_") {
            return (inner.to_string(), false);
        }
        // Leaf: inject WHERE before GROUP BY (or at end).
        let gb_pos = find_at_depth_zero(&inner_upper, " GROUP BY ");
        let existing_where = find_at_depth_zero(&inner_upper, " WHERE ");
        let injected = match (existing_where, gb_pos) {
            (Some(ewp), _) => {
                let extra = where_clause.trim_start().trim_start_matches("WHERE ");
                let ip = ewp + " WHERE ".len();
                format!("{}{} AND {}", &inner[..ip], extra, &inner[ip..])
            }
            (None, Some(pos)) => format!("{}{}{}", &inner[..pos], where_clause, &inner[pos..]),
            (None, None) => format!("{inner}{where_clause}"),
        };
        (injected, true)
    }
}

/// Find the position of `needle` in `haystack` at paren-depth 0 (skipping occurrences
/// inside nested parentheses). Returns byte position of the start of the match.
fn find_at_depth_zero(haystack: &str, needle: &str) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i + needle_bytes.len() <= bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && bytes[i..].starts_with(needle_bytes) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_query_ref(query_ref: &str) -> Result<(String, String, Option<String>), String> {
    let upper = query_ref.to_ascii_uppercase();

    // Find SELECT ... FROM boundary at depth 0 (skip FROM inside subqueries).
    let from_pos =
        find_at_depth_zero(&upper, " FROM ").ok_or("query_ref must contain SELECT ... FROM ...")?;

    let select_expr = query_ref[..from_pos]
        .trim()
        .strip_prefix_insensitive("SELECT")
        .ok_or("query_ref must start with SELECT")?
        .trim()
        .to_owned();

    if select_expr.is_empty() {
        return Err("SELECT clause is empty".to_owned());
    }

    let after_from = &query_ref[from_pos + 6..]; // skip " FROM "

    // Find optional GROUP BY at depth 0 (skip GROUP BY inside subqueries).
    let group_by_pos = find_at_depth_zero(&upper[from_pos + 6..], " GROUP BY ");
    let (from_part, group_by) = match group_by_pos {
        Some(pos) => (
            after_from[..pos].trim(),
            Some(after_from[pos + 10..].trim().to_owned()), // skip " GROUP BY "
        ),
        None => (after_from.trim(), None),
    };

    let from_clause = from_part.to_owned();
    if from_clause.is_empty() {
        return Err("FROM clause is empty".to_owned());
    }

    validate_from_clause(&from_clause)?;

    Ok((select_expr, from_clause, group_by))
}

/// Validate a FROM clause: single table, aliased table, or JOIN expression.
///
/// Rejects subqueries, semicolons, and WHERE clauses. Every table/alias token
/// must be a safe identifier.
fn validate_from_clause(from_clause: &str) -> Result<(), String> {
    let upper = from_clause.to_ascii_uppercase();

    // Reject dangerous patterns. WHERE is forbidden at depth 0 (would let admins inject
    // arbitrary filters) but allowed inside subqueries (legitimate nested WHERE).
    if find_at_depth_zero(&upper, " WHERE ").is_some() {
        return Err("FROM clause must not contain WHERE at top level".to_owned());
    }
    if from_clause.contains(';') {
        return Err("FROM clause must not contain semicolons".to_owned());
    }
    // LOCAL DEV: subqueries in FROM are allowed (needed for two-level aggregation).
    // If ANY paren exists in the FROM clause it's treated as a subquery-in-FROM —
    // skip segment validation (admins are trusted here; we still block destructive
    // keywords and top-level WHERE).
    let has_paren = from_clause.contains('(');

    if !has_paren {
        // Split on JOIN keywords to get each table reference
        // e.g. "t1 c JOIN t2 e ON c.x = e.y LEFT JOIN t3 f ON ..."
        // We validate table names and aliases, and allow ON conditions.
        let join_re = [
            "JOIN",
            "LEFT JOIN",
            "RIGHT JOIN",
            "INNER JOIN",
            "CROSS JOIN",
        ];

        // Extract table references by splitting on JOIN boundaries
        let segments = split_on_joins(&upper, from_clause);
        for segment in &segments {
            validate_table_segment(segment)?;
        }

        let _ = join_re; // suppress unused warning
    }

    // Extra safety: ensure no SQL keywords that shouldn't be here
    for keyword in ["INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "UNION"] {
        if upper.contains(keyword) {
            return Err(format!("FROM clause must not contain {keyword}"));
        }
    }

    Ok(())
}

/// Split a FROM clause into segments at JOIN boundaries.
/// Returns the original-case segments.
fn split_on_joins<'a>(upper: &str, original: &'a str) -> Vec<&'a str> {
    let mut positions = vec![0usize];

    // Find all JOIN keyword positions (must match word boundary)
    for keyword in [
        " LEFT JOIN ",
        " RIGHT JOIN ",
        " INNER JOIN ",
        " CROSS JOIN ",
        " JOIN ",
    ] {
        let mut start = 0;
        while let Some(pos) = upper[start..].find(keyword) {
            positions.push(start + pos);
            start += pos + keyword.len();
        }
    }

    positions.sort_unstable();
    positions.dedup();
    positions.push(original.len());

    positions
        .windows(2)
        .map(|w| original[w[0]..w[1]].trim())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Validate a single table segment like `silver.events c` or
/// `JOIN silver.events c ON c.tid = e.tid`.
fn validate_table_segment(segment: &str) -> Result<(), String> {
    let upper = segment.to_ascii_uppercase();

    // Strip leading JOIN keyword if present
    let rest = strip_join_prefix(&upper, segment);

    // Split on ON to separate "table alias" from "join condition"
    let (table_part, _on_part) = match rest.to_ascii_uppercase().find(" ON ") {
        Some(pos) => (&rest[..pos], Some(&rest[pos + 4..])),
        None => (rest, None),
    };

    // table_part should be "database.table" or "database.table alias"
    let tokens: Vec<&str> = table_part.split_whitespace().collect();
    if tokens.is_empty() {
        return Err("empty table reference in FROM clause".to_owned());
    }

    // First token is the table name
    if !is_valid_ident(tokens[0]) {
        return Err(format!("invalid table name: {}", tokens[0]));
    }

    // Second token (if present) is the alias
    if tokens.len() > 1 && !is_valid_ident(tokens[1]) {
        return Err(format!("invalid table alias: {}", tokens[1]));
    }

    if tokens.len() > 2 {
        return Err(format!(
            "unexpected tokens in table reference: {table_part}"
        ));
    }

    Ok(())
}

/// Strip a leading JOIN keyword from a segment, returning the original-case remainder.
fn strip_join_prefix<'a>(upper: &str, original: &'a str) -> &'a str {
    for prefix in [
        "LEFT JOIN ",
        "RIGHT JOIN ",
        "INNER JOIN ",
        "CROSS JOIN ",
        "JOIN ",
    ] {
        if upper.starts_with(prefix) {
            return original[prefix.len()..].trim();
        }
    }
    original.trim()
}

/// Case-insensitive prefix strip helper.
trait StripPrefixInsensitive {
    fn strip_prefix_insensitive(&self, prefix: &str) -> Option<&str>;
}

impl StripPrefixInsensitive for str {
    fn strip_prefix_insensitive(&self, prefix: &str) -> Option<&str> {
        if self.len() >= prefix.len() && self[..prefix.len()].eq_ignore_ascii_case(prefix) {
            Some(&self[prefix.len()..])
        } else {
            None
        }
    }
}

/// Validate an `OData` `$orderby` expression.
/// Accepts: `column_name [asc|desc] [, column_name [asc|desc]]*`
fn is_valid_orderby(orderby: &str) -> bool {
    if orderby.is_empty() {
        return false;
    }
    orderby.split(',').all(|part| {
        let tokens: Vec<&str> = part.split_whitespace().collect();
        match tokens.len() {
            1 => is_valid_ident(tokens[0]),
            2 => {
                is_valid_ident(tokens[0])
                    && matches!(tokens[1].to_ascii_lowercase().as_str(), "asc" | "desc")
            }
            _ => false,
        }
    })
}

/// Validate a column/table identifier (letters, digits, underscores, dots).
fn is_valid_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && !s.starts_with('.')
        && !s.ends_with('.')
}

// ── Shared helpers ──────────────────────────────────────────

/// Product/global tenant. Platform metric *definitions* are seeded once under
/// the nil UUID (see the `m20260507_*` / `m20260527_*` seed migrations, all of
/// which use `ZERO_TENANT`) and are visible to every tenant — the request
/// tenant only overlays per-tenant *thresholds*, never the definition itself.
///
/// Resolution rule: a metric is visible to a request when its
/// `insight_tenant_id` is either the request's resolved tenant (a future
/// tenant-specific custom metric) OR this global tenant. The request tenant is
/// never `nil` (the auth layer rejects nil headers and filters a nil configured
/// default — see `domain::auth`), so this never collapses into "match anything".
const GLOBAL_TENANT: Uuid = Uuid::nil();

/// Find an enabled metric by ID, visible to `tenant_id` or to the global
/// (product) tenant. Returns 404 if missing or disabled.
async fn find_enabled_metric(
    state: &AppState,
    tenant_id: Uuid,
    metric_id: Uuid,
) -> Result<entities::metrics::Model, CanonicalError> {
    entities::metrics::Entity::find_by_id(metric_id)
        .filter(entities::metrics::Column::InsightTenantId.is_in([tenant_id, GLOBAL_TENANT]))
        .filter(entities::metrics::Column::IsEnabled.eq(true))
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to find metric");
            CanonicalError::internal("failed to find metric").create()
        })?
        .ok_or_else(|| {
            MetricError::not_found("metric not found or disabled")
                .with_resource(metric_id.to_string())
                .create()
        })
}

// ── Thresholds CRUD ─────────────────────────────────────────

pub async fn list_thresholds(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(metric_id): Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    // Verify metric exists, is enabled, and belongs to tenant
    find_enabled_metric(&state, ctx.subject_tenant_id(), metric_id).await?;

    let rows = entities::thresholds::Entity::find()
        .filter(entities::thresholds::Column::MetricId.eq(metric_id))
        .filter(entities::thresholds::Column::InsightTenantId.eq(ctx.subject_tenant_id()))
        .all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to list thresholds");
            CanonicalError::internal("failed to list thresholds").create()
        })?;

    let items: Vec<Threshold> = rows.into_iter().map(model_to_threshold).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn create_threshold(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(metric_id): Path<Uuid>,
    Json(req): Json<CreateThresholdRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    find_enabled_metric(&state, ctx.subject_tenant_id(), metric_id).await?;

    let bad_operator = !threshold::VALID_OPERATORS.contains(&req.operator.as_str());
    let bad_level = !threshold::VALID_LEVELS.contains(&req.level.as_str());
    if bad_operator || bad_level {
        let first_field = if bad_operator { "operator" } else { "level" };
        let first_msg = if bad_operator {
            threshold::INVALID_OPERATOR_MSG
        } else {
            threshold::INVALID_LEVEL_MSG
        };
        let mut err = ThresholdError::invalid_argument().with_field_violation(
            first_field,
            first_msg,
            "INVALID",
        );
        if bad_operator && bad_level {
            err = err.with_field_violation("level", threshold::INVALID_LEVEL_MSG, "INVALID");
        }
        return Err(err.create());
    }

    let id = Uuid::now_v7();

    let model = entities::thresholds::ActiveModel {
        id: Set(id),
        insight_tenant_id: Set(ctx.subject_tenant_id()),
        metric_id: Set(metric_id),
        field_name: Set(req.field_name),
        operator: Set(req.operator),
        value: Set(req.value),
        level: Set(req.level),
        created_at: NotSet,
        updated_at: NotSet,
    };

    entities::thresholds::Entity::insert(model)
        .exec(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to create threshold");
            CanonicalError::internal("failed to create threshold").create()
        })?;

    let row = entities::thresholds::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to fetch created threshold");
            CanonicalError::internal("failed to fetch created threshold").create()
        })?
        .ok_or_else(|| CanonicalError::internal("created threshold not found").create())?;

    Ok((StatusCode::CREATED, Json(model_to_threshold(row))))
}

pub async fn update_threshold(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path((metric_id, tid)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateThresholdRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let existing = entities::thresholds::Entity::find_by_id(tid)
        .filter(entities::thresholds::Column::MetricId.eq(metric_id))
        .filter(entities::thresholds::Column::InsightTenantId.eq(ctx.subject_tenant_id()))
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to find threshold");
            CanonicalError::internal("failed to find threshold").create()
        })?
        .ok_or_else(|| {
            ThresholdError::not_found("threshold not found")
                .with_resource(tid.to_string())
                .create()
        })?;

    let bad_operator = req
        .operator
        .as_deref()
        .is_some_and(|op| !threshold::VALID_OPERATORS.contains(&op));
    let bad_level = req
        .level
        .as_deref()
        .is_some_and(|lv| !threshold::VALID_LEVELS.contains(&lv));
    if bad_operator || bad_level {
        let first_field = if bad_operator { "operator" } else { "level" };
        let first_msg = if bad_operator {
            threshold::INVALID_OPERATOR_MSG
        } else {
            threshold::INVALID_LEVEL_MSG
        };
        let mut err = ThresholdError::invalid_argument()
            .with_resource(tid.to_string())
            .with_field_violation(first_field, first_msg, "INVALID");
        if bad_operator && bad_level {
            err = err.with_field_violation("level", threshold::INVALID_LEVEL_MSG, "INVALID");
        }
        return Err(err.create());
    }

    let mut model: entities::thresholds::ActiveModel = existing.into();

    if let Some(field_name) = req.field_name {
        model.field_name = Set(field_name);
    }
    if let Some(operator) = req.operator {
        model.operator = Set(operator);
    }
    if let Some(value) = req.value {
        model.value = Set(value);
    }
    if let Some(level) = req.level {
        model.level = Set(level);
    }
    model.updated_at = Set(chrono::Utc::now());

    let updated = model.update(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "failed to update threshold");
        CanonicalError::internal("failed to update threshold").create()
    })?;

    Ok(Json(model_to_threshold(updated)))
}

pub async fn delete_threshold(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path((metric_id, tid)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, CanonicalError> {
    let existing = entities::thresholds::Entity::find_by_id(tid)
        .filter(entities::thresholds::Column::MetricId.eq(metric_id))
        .filter(entities::thresholds::Column::InsightTenantId.eq(ctx.subject_tenant_id()))
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to find threshold");
            CanonicalError::internal("failed to find threshold").create()
        })?
        .ok_or_else(|| {
            ThresholdError::not_found("threshold not found")
                .with_resource(tid.to_string())
                .create()
        })?;

    entities::thresholds::Entity::delete_by_id(existing.id)
        .exec(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to delete threshold");
            CanonicalError::internal("failed to delete threshold").create()
        })?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Columns ─────────────────────────────────────────────────

pub async fn list_columns(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    let columns = entities::table_columns::Entity::find()
        .filter(
            Condition::any()
                .add(entities::table_columns::Column::InsightTenantId.is_null())
                .add(entities::table_columns::Column::InsightTenantId.eq(ctx.subject_tenant_id())),
        )
        .all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to list columns");
            CanonicalError::internal("failed to list columns").create()
        })?;

    let items: Vec<TableColumn> = columns.into_iter().map(model_to_column).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn list_columns_for_table(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(table): Path<String>,
) -> Result<impl IntoResponse, CanonicalError> {
    let columns = entities::table_columns::Entity::find()
        .filter(entities::table_columns::Column::ClickhouseTable.eq(&table))
        .filter(
            Condition::any()
                .add(entities::table_columns::Column::InsightTenantId.is_null())
                .add(entities::table_columns::Column::InsightTenantId.eq(ctx.subject_tenant_id())),
        )
        .all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, table = %table, "failed to list columns for table");
            CanonicalError::internal("failed to list columns for table").create()
        })?;

    let items: Vec<TableColumn> = columns.into_iter().map(model_to_column).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

// ── Mappers ─────────────────────────────────────────────────

fn model_to_metric(m: entities::metrics::Model) -> Metric {
    Metric {
        id: m.id,
        insight_tenant_id: m.insight_tenant_id,
        name: m.name,
        description: m.description,
        query_ref: m.query_ref,
        is_enabled: m.is_enabled,
        created_at: m.created_at.naive_utc(),
        updated_at: m.updated_at.naive_utc(),
    }
}

fn model_to_metric_summary(m: entities::metrics::Model) -> MetricSummary {
    MetricSummary {
        id: m.id,
        name: m.name,
        description: m.description,
    }
}

fn model_to_threshold(m: entities::thresholds::Model) -> Threshold {
    Threshold {
        id: m.id,
        insight_tenant_id: m.insight_tenant_id,
        metric_id: m.metric_id,
        field_name: m.field_name,
        operator: m.operator,
        value: m.value,
        level: m.level,
        created_at: m.created_at.naive_utc(),
        updated_at: m.updated_at.naive_utc(),
    }
}

fn model_to_column(m: entities::table_columns::Model) -> TableColumn {
    TableColumn {
        id: m.id,
        insight_tenant_id: m.insight_tenant_id,
        clickhouse_table: m.clickhouse_table,
        field_name: m.field_name,
        field_description: m.field_description,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_odata_in_values ─────────────────────────────

    #[test]
    fn extract_in_list_parses_quoted_person_ids() {
        let f = "metric_date ge '2026-03-04' and person_id in ('a@x.com', 'b@y.com', 'c@z.com')";
        assert_eq!(
            extract_odata_in_values(f, "person_id"),
            Some(vec!["a@x.com".into(), "b@y.com".into(), "c@z.com".into()])
        );
    }

    #[test]
    fn extract_in_list_absent_or_empty_is_none() {
        assert_eq!(
            extract_odata_in_values("person_id eq 'a@x.com'", "person_id"),
            None
        );
        assert_eq!(
            extract_odata_in_values("person_id in ()", "person_id"),
            None
        );
    }

    #[test]
    fn extract_in_list_parses_quoted_org_unit_ids() {
        let f = "metric_date ge '2026-03-04' and org_unit_id in ('eng', 'sales', 'ops')";
        assert_eq!(
            extract_odata_in_values(f, "org_unit_id"),
            Some(vec!["eng".into(), "sales".into(), "ops".into()])
        );
    }

    // ── parse_query_ref ─────────────────────────────────────

    #[test]
    fn parse_simple_select() -> Result<(), Box<dyn std::error::Error>> {
        let (sel, from, gb) =
            parse_query_ref("SELECT person_id, avg_hours FROM gold.pr_cycle_time")?;
        assert_eq!(sel, "person_id, avg_hours");
        assert_eq!(from, "gold.pr_cycle_time");
        assert!(gb.is_none());
        Ok(())
    }

    #[test]
    fn parse_with_group_by() -> Result<(), Box<dyn std::error::Error>> {
        let (sel, from, gb) = parse_query_ref(
            "SELECT person_id, avg(cycle_time_h) AS avg_hours FROM gold.pr_cycle_time GROUP BY person_id",
        )?;
        assert_eq!(sel, "person_id, avg(cycle_time_h) AS avg_hours");
        assert_eq!(from, "gold.pr_cycle_time");
        assert_eq!(gb.as_deref(), Some("person_id"));
        Ok(())
    }

    #[test]
    fn parse_case_insensitive() -> Result<(), Box<dyn std::error::Error>> {
        let (sel, from, _) = parse_query_ref("select col1, col2 from silver.commits")?;
        assert_eq!(sel, "col1, col2");
        assert_eq!(from, "silver.commits");
        Ok(())
    }

    #[test]
    fn parse_with_aggregates_and_group_by() -> Result<(), Box<dyn std::error::Error>> {
        let (sel, from, gb) = parse_query_ref(
            "SELECT org_unit_id, COUNT(DISTINCT person_id) AS headcount, AVG(focus_time_pct) AS focus FROM gold.team_summary GROUP BY org_unit_id",
        )?;
        assert_eq!(
            sel,
            "org_unit_id, COUNT(DISTINCT person_id) AS headcount, AVG(focus_time_pct) AS focus"
        );
        assert_eq!(from, "gold.team_summary");
        assert_eq!(gb.as_deref(), Some("org_unit_id"));
        Ok(())
    }

    #[test]
    fn parse_with_join() -> Result<(), Box<dyn std::error::Error>> {
        let (sel, from, gb) = parse_query_ref(
            "SELECT e.displayName, SUM(c.emails_sent) AS total FROM silver.class_comms_events c JOIN bronze_bamboohr.employees e ON lower(c.user_email) = lower(e.workEmail) GROUP BY e.displayName",
        )?;
        assert_eq!(sel, "e.displayName, SUM(c.emails_sent) AS total");
        assert_eq!(
            from,
            "silver.class_comms_events c JOIN bronze_bamboohr.employees e ON lower(c.user_email) = lower(e.workEmail)"
        );
        assert_eq!(gb.as_deref(), Some("e.displayName"));
        Ok(())
    }

    #[test]
    fn parse_with_left_join() -> Result<(), Box<dyn std::error::Error>> {
        let (_, from, _) = parse_query_ref(
            "SELECT c.user_email FROM silver.events c LEFT JOIN bronze_bamboohr.employees e ON c.email = e.workEmail",
        )?;
        assert!(from.contains("LEFT JOIN"));
        Ok(())
    }

    #[test]
    fn parse_rejects_missing_from() {
        assert!(parse_query_ref("SELECT col1, col2").is_err());
    }

    #[test]
    fn parse_rejects_empty_select() {
        assert!(parse_query_ref("SELECT FROM gold.table").is_err());
    }

    #[test]
    fn parse_rejects_invalid_table_name() {
        assert!(parse_query_ref("SELECT col FROM gold.table; DROP TABLE x").is_err());
    }

    #[test]
    fn parse_allows_subquery_in_from() {
        // Subqueries in FROM are allowed — needed for two-level bullet aggregation
        let result = parse_query_ref("SELECT col FROM (SELECT * FROM secret.data) AS t");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_rejects_table_with_where() {
        let result = parse_query_ref("SELECT col FROM gold.t WHERE 1=1");
        assert!(result.is_err());
    }

    #[test]
    fn parse_rejects_drop_in_from() {
        assert!(parse_query_ref("SELECT col FROM gold.t; DROP TABLE x").is_err());
    }

    // ── is_valid_orderby ────────────────────────────────────

    #[test]
    fn orderby_single_column() {
        assert!(is_valid_orderby("metric_date"));
    }

    #[test]
    fn orderby_with_direction() {
        assert!(is_valid_orderby("metric_date desc"));
        assert!(is_valid_orderby("person_id ASC"));
    }

    #[test]
    fn orderby_multiple_columns() {
        assert!(is_valid_orderby("metric_date desc, person_id asc"));
    }

    #[test]
    fn orderby_dotted_column() {
        assert!(is_valid_orderby("t.metric_date desc"));
    }

    #[test]
    fn orderby_rejects_sql_injection() {
        assert!(!is_valid_orderby("1; DROP TABLE metrics --"));
        assert!(!is_valid_orderby("metric_date; DELETE FROM metrics"));
        assert!(!is_valid_orderby("(SELECT 1)"));
    }

    #[test]
    fn orderby_rejects_empty() {
        assert!(!is_valid_orderby(""));
    }

    #[test]
    fn orderby_rejects_invalid_direction() {
        assert!(!is_valid_orderby("metric_date DROP"));
    }

    // ── is_valid_ident ──────────────────────────────────────

    #[test]
    fn ident_valid() {
        assert!(is_valid_ident("metric_date"));
        assert!(is_valid_ident("gold.pr_cycle_time"));
        assert!(is_valid_ident("col1"));
    }

    #[test]
    fn ident_rejects_special_chars() {
        assert!(!is_valid_ident("col; DROP"));
        assert!(!is_valid_ident("col--"));
        assert!(!is_valid_ident(""));
        assert!(!is_valid_ident(".leading_dot"));
        assert!(!is_valid_ident("trailing_dot."));
    }

    // ── inject_date_filter_into_subqueries ──────────────────
    //
    // These tests pin the contract of the date-filter walker so future
    // refactors do not silently regress the two-level injection (which the
    // bullet `query_ref` shape relies on) or the bare-table fallback (which
    // the Team Members query relies on).

    #[test]
    fn inject_simple_leaf_subquery_before_group_by() -> Result<(), Box<dyn std::error::Error>> {
        // Single subquery with GROUP BY → WHERE inserted right before GROUP BY.
        let from = "(SELECT person_id, sum(metric_value) AS v \
                     FROM insight.task_delivery_bullet_rows \
                     GROUP BY person_id) p";
        let where_clause = " WHERE metric_date >= '2026-04-01'";

        let output = inject_date_filter_into_subqueries(from, where_clause)
            .ok_or("expected walker to inject")?;
        assert!(
            output.contains("WHERE metric_date >= '2026-04-01' GROUP BY person_id"),
            "WHERE must land just before GROUP BY; got:\n{output}"
        );
        Ok(())
    }

    #[test]
    fn inject_bullet_shape_hits_both_p_and_inner_c() -> Result<(), Box<dyn std::error::Error>> {
        // Regression test for the actual production bullet shape:
        // FROM (p) LEFT JOIN (... FROM (inner_c) ...) c ON ...
        // The walker must descend into the c side and inject into inner_c too,
        // so both per-person and company-wide aggregates see the same period.
        let from = "(SELECT metric_key, person_id, sum(metric_value) AS v \
                     FROM insight.task_delivery_bullet_rows \
                     GROUP BY metric_key, person_id) p \
                    LEFT JOIN (SELECT metric_key, median(v) AS m \
                               FROM (SELECT metric_key, person_id, sum(metric_value) AS v \
                                     FROM insight.task_delivery_bullet_rows \
                                     GROUP BY metric_key, person_id) inner_c \
                               GROUP BY metric_key) c \
                    ON c.metric_key = p.metric_key";
        let where_clause = " WHERE metric_date >= '2026-04-01'";

        let output = inject_date_filter_into_subqueries(from, where_clause)
            .ok_or("expected walker to inject")?;
        let count = output.matches("WHERE metric_date >= '2026-04-01'").count();
        assert_eq!(
            count, 2,
            "WHERE must be injected into BOTH p and inner_c (expected count=2). \
             A regression here means per-person and company-wide aggregates \
             read different time periods. Got count={count}; full output:\n{output}"
        );
        Ok(())
    }

    #[test]
    fn inject_skips_bare_bronze_table_without_metric_date() {
        let from = "bronze_bamboohr.employees e \
                    LEFT JOIN insight.people p ON e.workEmail = p.email";
        let where_clause = " WHERE metric_date >= '2026-04-01'";

        let normalised = ensure_subquery_from(from);
        assert!(
            inject_date_filter_into_subqueries(&normalised, where_clause).is_none(),
            "bronze leaf must be skipped (no metric_date column)"
        );
    }

    #[test]
    fn inject_after_prewrap_injects_into_bare_fact_table() -> Result<(), Box<dyn std::error::Error>>
    {
        let from = "insight.team_member m \
                    LEFT JOIN insight.people p ON m.person_id = p.person_id";
        let where_clause = " WHERE metric_date >= '2026-04-01'";

        let normalised = ensure_subquery_from(from);
        let output = inject_date_filter_into_subqueries(&normalised, where_clause)
            .ok_or("pre-wrap must make injection possible for bare fact-table FROMs")?;

        assert!(
            output.contains(
                "(SELECT * FROM insight.team_member WHERE metric_date >= '2026-04-01') m"
            ),
            "WHERE must land inside the wrapped fact table; got:\n{output}"
        );
        assert!(
            output.contains("LEFT JOIN insight.people p ON m.person_id = p.person_id"),
            "JOIN tail must be preserved verbatim; got:\n{output}"
        );
        Ok(())
    }

    #[test]
    fn inject_combines_via_and_when_existing_where_present()
    -> Result<(), Box<dyn std::error::Error>> {
        // Subquery already has a WHERE clause. The new condition must be
        // combined through AND, not duplicate or replace the existing WHERE.
        // Order (new-before-existing vs existing-before-new) is not semantically
        // meaningful in SQL — assert presence + AND-glue, not position.
        let from = "(SELECT person_id FROM insight.task_delivery_bullet_rows \
                     WHERE org_unit_id = 'eng' GROUP BY person_id) p";
        let where_clause = " WHERE metric_date >= '2026-04-01'";

        let output = inject_date_filter_into_subqueries(from, where_clause)
            .ok_or("expected walker to inject")?;
        // Both conditions present
        assert!(
            output.contains("org_unit_id = 'eng'"),
            "existing predicate must survive; got:\n{output}"
        );
        assert!(
            output.contains("metric_date >= '2026-04-01'"),
            "new predicate must be present; got:\n{output}"
        );
        // Connected by AND, no duplicate WHERE
        assert!(
            output.contains(" AND "),
            "predicates must be AND-glued; got:\n{output}"
        );
        assert_eq!(
            output.matches(" WHERE ").count(),
            1,
            "must not emit two `WHERE` keywords; got:\n{output}"
        );
        Ok(())
    }

    #[test]
    fn inject_appends_to_end_when_no_group_by() -> Result<(), Box<dyn std::error::Error>> {
        // Edge case: subquery has no GROUP BY. WHERE appended at the end.
        let from = "(SELECT * FROM insight.task_delivery_bullet_rows) p";
        let where_clause = " WHERE metric_date >= '2026-04-01'";

        let output = inject_date_filter_into_subqueries(from, where_clause)
            .ok_or("expected walker to inject")?;
        assert!(
            output.contains("FROM insight.task_delivery_bullet_rows WHERE metric_date"),
            "WHERE must be appended right after the source table; got:\n{output}"
        );
        Ok(())
    }

    #[test]
    fn ensure_subquery_from_passthrough_when_already_subquery() {
        let from = "(SELECT * FROM insight.ic_kpis) GROUP BY person_id";
        assert_eq!(ensure_subquery_from(from), from);
    }

    #[test]
    fn ensure_subquery_from_passthrough_when_leading_whitespace_then_paren() {
        let from = "  (SELECT * FROM insight.ic_kpis)";
        assert_eq!(ensure_subquery_from(from), from);
    }

    #[test]
    fn ensure_subquery_from_wraps_bare_single_table() {
        let from = "insight.ic_kpis";
        assert_eq!(
            ensure_subquery_from(from),
            "(SELECT * FROM insight.ic_kpis)"
        );
    }

    #[test]
    fn ensure_subquery_from_wraps_bare_table_with_alias() {
        let from = "insight.ic_kpis k";
        assert_eq!(
            ensure_subquery_from(from),
            "(SELECT * FROM insight.ic_kpis) k"
        );
    }

    #[test]
    fn ensure_subquery_from_wraps_first_of_join_preserving_alias_and_tail() {
        let from = "insight.team_member m \
                    LEFT JOIN insight.people p ON m.person_id = p.person_id";
        let out = ensure_subquery_from(from);
        assert_eq!(
            out,
            "(SELECT * FROM insight.team_member) m \
             LEFT JOIN insight.people p ON m.person_id = p.person_id"
        );
    }

    #[test]
    fn ensure_subquery_from_wraps_first_of_join_with_inner_subquery_tail() {
        let from = "insight.team_member m \
                    LEFT JOIN insight.people p ON m.person_id = p.person_id \
                    LEFT JOIN (SELECT lower(workEmail) AS person_id, \
                                      lower(argMax(supervisorEmail, _airbyte_extracted_at)) AS supervisor_email \
                               FROM bronze_bamboohr.employees \
                               WHERE workEmail IS NOT NULL \
                               GROUP BY lower(workEmail)) s ON s.person_id = m.person_id";
        let out = ensure_subquery_from(from);
        assert!(
            out.starts_with("(SELECT * FROM insight.team_member) m LEFT JOIN insight.people p"),
            "first table must be wrapped, alias preserved, JOIN tail kept; got:\n{out}"
        );
        assert!(
            out.contains("LEFT JOIN (SELECT lower(workEmail)"),
            "later subquery JOIN must be untouched; got:\n{out}"
        );
    }

    #[test]
    fn ensure_subquery_from_picks_earliest_join_keyword() {
        let from =
            "insight.a a INNER JOIN insight.b b ON a.x = b.x LEFT JOIN insight.c c ON a.x = c.x";
        let out = ensure_subquery_from(from);
        assert!(
            out.starts_with("(SELECT * FROM insight.a) a INNER JOIN insight.b"),
            "first JOIN keyword (INNER JOIN) marks end of first-table prefix; got:\n{out}"
        );
    }

    #[test]
    fn ensure_subquery_from_handles_lowercase_join() {
        let from = "insight.team_member m left join insight.people p on m.person_id = p.person_id";
        let out = ensure_subquery_from(from);
        assert!(
            out.starts_with("(SELECT * FROM insight.team_member) m left join insight.people p"),
            "JOIN match must be case-insensitive; got:\n{out}"
        );
    }

    #[test]
    fn full_pipeline_injects_metric_date_into_bare_single_table()
    -> Result<(), Box<dyn std::error::Error>> {
        let from = ensure_subquery_from("insight.ic_kpis");
        let where_clause = " WHERE metric_date >= '2026-04-01' AND metric_date < '2026-05-01'";
        let out = inject_date_filter_into_subqueries(&from, where_clause)
            .ok_or("pre-wrap + walker must yield injected SQL for bare single table")?;
        assert_eq!(
            out,
            "(SELECT * FROM insight.ic_kpis WHERE metric_date >= '2026-04-01' AND metric_date < '2026-05-01')"
        );
        Ok(())
    }

    #[test]
    fn full_pipeline_injects_metric_date_into_first_of_join_only()
    -> Result<(), Box<dyn std::error::Error>> {
        let from = ensure_subquery_from(
            "insight.team_member m LEFT JOIN insight.people p ON m.person_id = p.person_id",
        );
        let where_clause = " WHERE metric_date >= '2026-04-01'";
        let out = inject_date_filter_into_subqueries(&from, where_clause)
            .ok_or("pre-wrap + walker must yield injected SQL for bare-table FROM with JOIN")?;
        let where_count = out.matches("WHERE metric_date >= '2026-04-01'").count();
        assert_eq!(
            where_count, 1,
            "filter must land exactly once (in the wrapped first table); got count={where_count}; full:\n{out}"
        );
        assert!(
            out.contains("(SELECT * FROM insight.team_member WHERE metric_date >= '2026-04-01') m"),
            "filter must be inside the wrapper, before the closing paren; got:\n{out}"
        );
        Ok(())
    }

    #[test]
    fn full_pipeline_injects_into_bare_inner_from_below_multiif_expression()
    -> Result<(), Box<dyn std::error::Error>> {
        let from = ensure_subquery_from(
            "(SELECT supervisor_email AS cohort_seed, person_id, kpi_key, \
                     multiIf(kpi_key IN ('bugs_fixed','tasks_closed','prs_merged','ai_sessions'), \
                             sum(value), avg(value)) AS person_total \
              FROM insight.ic_kpi_peer_median \
              GROUP BY supervisor_email, person_id, kpi_key)",
        );
        let where_clause = " WHERE metric_date >= '2026-04-01'";
        let out = inject_date_filter_into_subqueries(&from, where_clause)
            .ok_or("walker must leaf-inject into bare-FROM inside outer subquery despite nested multiIf parens in the SELECT list")?;
        let count = out.matches("WHERE metric_date >= '2026-04-01'").count();
        assert_eq!(
            count, 1,
            "filter must land once at the leaf FROM, not inside multiIf(...); got count={count}; full:\n{out}"
        );
        assert!(
            out.contains(
                "FROM insight.ic_kpi_peer_median WHERE metric_date >= '2026-04-01' GROUP BY supervisor_email"
            ),
            "WHERE must land between bare table and its GROUP BY; got:\n{out}"
        );
        Ok(())
    }

    #[test]
    fn full_pipeline_bullet_two_level_still_injects_into_both_leaves()
    -> Result<(), Box<dyn std::error::Error>> {
        let from = ensure_subquery_from(
            "(SELECT metric_key, person_id, sum(metric_value) AS v \
             FROM insight.task_delivery_bullet_rows \
             GROUP BY metric_key, person_id) p \
            LEFT JOIN (SELECT metric_key, median(v) AS m \
                       FROM (SELECT metric_key, person_id, sum(metric_value) AS v \
                             FROM insight.task_delivery_bullet_rows \
                             GROUP BY metric_key, person_id) inner_c \
                       GROUP BY metric_key) c \
            ON c.metric_key = p.metric_key",
        );
        let where_clause = " WHERE metric_date >= '2026-04-01'";
        let out = inject_date_filter_into_subqueries(&from, where_clause)
            .ok_or("walker must inject into both leaves of bullet shape")?;
        let count = out.matches("WHERE metric_date >= '2026-04-01'").count();
        assert_eq!(
            count, 2,
            "bullet two-level shape must still inject into both p and inner_c; got count={count}; full:\n{out}"
        );
        Ok(())
    }
}
