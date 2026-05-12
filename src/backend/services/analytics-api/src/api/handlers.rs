//! Route handlers.

use std::fmt::Write as _;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sea_orm::{ActiveModelTrait, ColumnTrait, Condition, EntityTrait, NotSet, QueryFilter, Set};
use uuid::Uuid;

use super::AppState;
use crate::auth::SecurityContext;
use crate::domain::metric::{
    CreateMetricRequest, Metric, MetricSummary, TableColumn, UpdateMetricRequest,
};
use crate::domain::query::{PageInfo, QueryRequest, QueryResponse};
use crate::domain::threshold;
use crate::domain::threshold::{CreateThresholdRequest, Threshold, UpdateThresholdRequest};
use crate::infra::db::entities;

// ── Error type (RFC 9457 Problem Details) ───────────────────

/// API error that serializes as RFC 9457 `application/problem+json`.
pub struct ApiError {
    status: StatusCode,
    error_type: &'static str,
    title: &'static str,
    detail: String,
}

impl ApiError {
    fn bad_request(error_type: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error_type,
            title: "Bad Request",
            detail: detail.into(),
        }
    }

    fn not_found(error_type: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            error_type,
            title: "Not Found",
            detail: detail.into(),
        }
    }

    fn internal(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error_type: "urn:insight:error:internal",
            title: "Internal Server Error",
            detail: detail.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "type": self.error_type,
            "title": self.title,
            "status": self.status.as_u16(),
            "detail": self.detail,
        });
        (
            self.status,
            [(axum::http::header::CONTENT_TYPE, "application/problem+json")],
            Json(body),
        )
            .into_response()
    }
}

// ── Health ──────────────────────────────────────────────────

pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "healthy" }))
}

// ── Person lookup ──────────────────────────────────────────

pub async fn get_person(
    State(state): State<Arc<AppState>>,
    Path(email): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if !state.identity.is_configured() {
        return Err(ApiError::internal(
            "identity resolution service not configured",
        ));
    }

    let person = state.identity.get_person(&email).await.map_err(|e| {
        tracing::error!(error = %e, email = %email, "identity resolution request failed");
        ApiError::internal("identity resolution unavailable")
    })?;

    match person {
        Some(p) => {
            Ok(Json(serde_json::to_value(p).map_err(|_| {
                ApiError::internal("failed to serialize person")
            })?))
        }
        None => Err(ApiError::not_found(
            "urn:insight:error:person_not_found",
            format!("person with email '{email}' not found"),
        )),
    }
}

// ── Metrics CRUD ────────────────────────────────────────────

pub async fn list_metrics(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = entities::metrics::Entity::find()
        .filter(entities::metrics::Column::InsightTenantId.eq(ctx.insight_tenant_id))
        .filter(entities::metrics::Column::IsEnabled.eq(true))
        .all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to list metrics");
            ApiError::internal("failed to list metrics")
        })?;

    let items: Vec<MetricSummary> = rows.into_iter().map(model_to_metric_summary).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn get_metric(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let row = find_enabled_metric(&state, ctx.insight_tenant_id, id).await?;
    Ok(Json(model_to_metric(row)))
}

pub async fn create_metric(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<CreateMetricRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Validate query_ref on write — reject malformed definitions early
    parse_query_ref(&req.query_ref)
        .map_err(|e| ApiError::bad_request("urn:insight:error:invalid_query_ref", e))?;

    let id = Uuid::now_v7();

    let model = entities::metrics::ActiveModel {
        id: Set(id),
        insight_tenant_id: Set(ctx.insight_tenant_id),
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
            ApiError::internal("failed to create metric")
        })?;

    let row = entities::metrics::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(|_| ApiError::internal("failed to fetch created metric"))?
        .ok_or_else(|| ApiError::internal("created metric not found"))?;

    Ok((StatusCode::CREATED, Json(model_to_metric(row))))
}

pub async fn update_metric(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateMetricRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let existing = find_enabled_metric(&state, ctx.insight_tenant_id, id).await?;
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
        parse_query_ref(&query_ref)
            .map_err(|e| ApiError::bad_request("urn:insight:error:invalid_query_ref", e))?;
        model.query_ref = Set(query_ref);
    }
    if let Some(enabled) = req.is_enabled {
        model.is_enabled = Set(enabled);
    }
    model.updated_at = Set(chrono::Utc::now());

    let updated = model.update(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "failed to update metric");
        ApiError::internal("failed to update metric")
    })?;

    Ok(Json(model_to_metric(updated)))
}

pub async fn delete_metric(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let existing = find_enabled_metric(&state, ctx.insight_tenant_id, id).await?;

    let mut model: entities::metrics::ActiveModel = existing.into();
    model.is_enabled = Set(false);
    model.updated_at = Set(chrono::Utc::now());
    model.update(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "failed to soft-delete metric");
        ApiError::internal("failed to soft-delete metric")
    })?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Query ───────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
pub async fn query_metric(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
    Json(req): Json<QueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // 1. Load metric definition (must be enabled)
    let metric = find_enabled_metric(&state, ctx.insight_tenant_id, id).await?;

    // 2. Validate $top
    let top = req.top.clamp(1, 200);

    // 4. Build ClickHouse query from structured metric fields.
    //
    // The engine always controls FROM and WHERE — insight_tenant_id is
    // always injected for tenant isolation. Admins never control WHERE.
    //
    // Person ID resolution: if identity_resolution_url is configured,
    // person_ids from $filter would be resolved to source aliases via
    // the Identity Resolution API. For MVP, the service is not deployed —
    // person_ids from the JWT subject_id are used directly against
    // ClickHouse (Gold tables already have resolved person_id columns).
    //
    // TODO: Full implementation should also:
    // - Validate org_unit_id from $filter against AccessScope (IDOR prevention)
    // - Resolve person_ids when identity_resolution_url is set
    // - Parse $select to restrict returned columns
    // - Implement cursor-based pagination (decode $skip → keyset)

    let (select_expr, from_clause, group_by) = parse_query_ref(&metric.query_ref).map_err(|e| {
        tracing::error!(error = %e, query_ref = %metric.query_ref, "invalid query_ref");
        ApiError::internal("metric has invalid query_ref")
    })?;

    // Allow $select to override the columns from query_ref
    let select_expr = match &req.select {
        Some(sel) if !sel.is_empty() => {
            if !sel.split(',').all(|col| is_valid_ident(col.trim())) {
                return Err(ApiError::bad_request(
                    "urn:insight:error:invalid_select",
                    format!("invalid $select: {sel}"),
                ));
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
    // selected period. Outer person_id/org_unit_id filters still apply post-aggregate.
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
        if (date_from.is_some() || date_to.is_some()) && from_clause.trim_start().starts_with('(') {
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
            (
                inject_where_into_first_subquery(&from_clause, &where_inner)
                    .unwrap_or_else(|| from_clause.clone()),
                true,
            )
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
            if let Some(date_from) = extract_odata_value(filter, "metric_date", "ge") {
                sql.push_str(" AND metric_date >= ?");
                params.push(date_from);
            }
            if let Some(date_to) = extract_odata_value(filter, "metric_date", "lt") {
                sql.push_str(" AND metric_date < ?");
                params.push(date_to);
            } else if let Some(date_to) = extract_odata_value(filter, "metric_date", "le") {
                sql.push_str(" AND metric_date <= ?");
                params.push(date_to);
            }
        }
        // Person filter — use person_id directly (no Identity Resolution for MVP).
        // Gold tables have a resolved person_id column; Silver tables would need
        // alias resolution via Identity Resolution API when it's available.
        if let Some(person_id) = extract_odata_value(filter, "person_id", "eq") {
            sql.push_str(" AND person_id = ?");
            params.push(person_id);
        }
        // Org-unit filter — used by Team View to scope to a single team.
        if let Some(org_unit_id) = extract_odata_value(filter, "org_unit_id", "eq") {
            sql.push_str(" AND org_unit_id = ?");
            params.push(org_unit_id);
        }
        // Drill filter — used by IC Dashboard drill modal.
        if let Some(drill_id) = extract_odata_value(filter, "drill_id", "eq") {
            sql.push_str(" AND drill_id = ?");
            params.push(drill_id);
        }
    }

    // Apply GROUP BY from parsed query_ref
    if let Some(ref gb) = group_by {
        let _ = write!(sql, " GROUP BY {gb}");
    }

    // Apply $orderby — validate against identifier pattern to prevent injection
    if let Some(ref orderby) = req.orderby {
        if !is_valid_orderby(orderby) {
            return Err(ApiError::bad_request(
                "urn:insight:error:invalid_order_by",
                format!("invalid $orderby: {orderby}"),
            ));
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
        ApiError::internal("query execution failed")
    })?;

    let raw_bytes = cursor.collect().await.map_err(|e| {
        tracing::error!(error = %e, sql = %sql, "ClickHouse fetch failed");
        ApiError::internal("query execution failed")
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
                ApiError::internal("failed to parse query results")
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

    let response = QueryResponse {
        items,
        page_info: PageInfo {
            has_next,
            cursor: None,
        },
    };

    Ok(Json(response))
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
/// Inject `where_clause` (` WHERE ...`) into every *leafmost* `(SELECT ...)` subquery
/// inside `from_clause`. A leaf subquery is one whose own FROM is a table (not another
/// subquery). This guarantees the `metric_date` filter lands at the level where the raw
/// table with a `metric_date` column is actually read. JOIN branches and nested
/// subqueries are recursed into; the filter is applied only at the innermost SELECT.
fn inject_where_into_first_subquery(from_clause: &str, where_clause: &str) -> Option<String> {
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

/// Find an enabled metric by ID and tenant. Returns 404 if missing or disabled.
async fn find_enabled_metric(
    state: &AppState,
    tenant_id: Uuid,
    metric_id: Uuid,
) -> Result<entities::metrics::Model, ApiError> {
    entities::metrics::Entity::find_by_id(metric_id)
        .filter(entities::metrics::Column::InsightTenantId.eq(tenant_id))
        .filter(entities::metrics::Column::IsEnabled.eq(true))
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to find metric");
            ApiError::internal("failed to find metric")
        })?
        .ok_or_else(|| {
            ApiError::not_found(
                "urn:insight:error:metric_not_found",
                "metric not found or disabled",
            )
        })
}

// ── Thresholds CRUD ─────────────────────────────────────────

pub async fn list_thresholds(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(metric_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    // Verify metric exists, is enabled, and belongs to tenant
    find_enabled_metric(&state, ctx.insight_tenant_id, metric_id).await?;

    let rows = entities::thresholds::Entity::find()
        .filter(entities::thresholds::Column::MetricId.eq(metric_id))
        .filter(entities::thresholds::Column::InsightTenantId.eq(ctx.insight_tenant_id))
        .all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to list thresholds");
            ApiError::internal("failed to list thresholds")
        })?;

    let items: Vec<Threshold> = rows.into_iter().map(model_to_threshold).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn create_threshold(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(metric_id): Path<Uuid>,
    Json(req): Json<CreateThresholdRequest>,
) -> Result<impl IntoResponse, ApiError> {
    find_enabled_metric(&state, ctx.insight_tenant_id, metric_id).await?;

    threshold::validate_threshold(&req.operator, &req.level)
        .map_err(|e| ApiError::bad_request("urn:insight:error:invalid_threshold", e))?;

    let id = Uuid::now_v7();

    let model = entities::thresholds::ActiveModel {
        id: Set(id),
        insight_tenant_id: Set(ctx.insight_tenant_id),
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
            ApiError::internal("failed to create threshold")
        })?;

    let row = entities::thresholds::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(|_| ApiError::internal("failed to fetch created threshold"))?
        .ok_or_else(|| ApiError::internal("created threshold not found"))?;

    Ok((StatusCode::CREATED, Json(model_to_threshold(row))))
}

pub async fn update_threshold(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path((metric_id, tid)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateThresholdRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let existing = entities::thresholds::Entity::find_by_id(tid)
        .filter(entities::thresholds::Column::MetricId.eq(metric_id))
        .filter(entities::thresholds::Column::InsightTenantId.eq(ctx.insight_tenant_id))
        .one(&state.db)
        .await
        .map_err(|_| ApiError::internal("failed to find threshold"))?
        .ok_or_else(|| {
            ApiError::not_found(
                "urn:insight:error:threshold_not_found",
                "threshold not found",
            )
        })?;

    let mut model: entities::thresholds::ActiveModel = existing.into();

    if let Some(field_name) = req.field_name {
        model.field_name = Set(field_name);
    }
    if let Some(operator) = req.operator {
        if !threshold::VALID_OPERATORS.contains(&operator.as_str()) {
            return Err(ApiError::bad_request(
                "urn:insight:error:invalid_threshold",
                "invalid operator",
            ));
        }
        model.operator = Set(operator);
    }
    if let Some(value) = req.value {
        model.value = Set(value);
    }
    if let Some(level) = req.level {
        if !threshold::VALID_LEVELS.contains(&level.as_str()) {
            return Err(ApiError::bad_request(
                "urn:insight:error:invalid_threshold",
                "invalid level",
            ));
        }
        model.level = Set(level);
    }
    model.updated_at = Set(chrono::Utc::now());

    let updated = model.update(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "failed to update threshold");
        ApiError::internal("failed to update threshold")
    })?;

    Ok(Json(model_to_threshold(updated)))
}

pub async fn delete_threshold(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path((metric_id, tid)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, ApiError> {
    let existing = entities::thresholds::Entity::find_by_id(tid)
        .filter(entities::thresholds::Column::MetricId.eq(metric_id))
        .filter(entities::thresholds::Column::InsightTenantId.eq(ctx.insight_tenant_id))
        .one(&state.db)
        .await
        .map_err(|_| ApiError::internal("failed to find threshold"))?
        .ok_or_else(|| {
            ApiError::not_found(
                "urn:insight:error:threshold_not_found",
                "threshold not found",
            )
        })?;

    entities::thresholds::Entity::delete_by_id(existing.id)
        .exec(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to delete threshold");
            ApiError::internal("failed to delete threshold")
        })?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Columns ─────────────────────────────────────────────────

pub async fn list_columns(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, ApiError> {
    let columns = entities::table_columns::Entity::find()
        .filter(
            Condition::any()
                .add(entities::table_columns::Column::InsightTenantId.is_null())
                .add(entities::table_columns::Column::InsightTenantId.eq(ctx.insight_tenant_id)),
        )
        .all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to list columns");
            ApiError::internal("failed to list columns")
        })?;

    let items: Vec<TableColumn> = columns.into_iter().map(model_to_column).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn list_columns_for_table(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(table): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let columns = entities::table_columns::Entity::find()
        .filter(entities::table_columns::Column::ClickhouseTable.eq(&table))
        .filter(
            Condition::any()
                .add(entities::table_columns::Column::InsightTenantId.is_null())
                .add(entities::table_columns::Column::InsightTenantId.eq(ctx.insight_tenant_id)),
        )
        .all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to list columns for table");
            ApiError::internal("failed to list columns for table")
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
}
