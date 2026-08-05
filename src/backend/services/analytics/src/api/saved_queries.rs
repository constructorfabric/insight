//! Saved-query CRUD + run handlers — `/v1/queries*` (#1965).
//!
//! CRUD is plain metadata over the `saved_queries` service-DB table, mirroring
//! the metric CRUD in [`super::handlers`]. Every request is tenant-scoped from
//! the session `SecurityContext`. The `sql` is validated by the single-SELECT
//! gate on create, update, and run. Only `/run` reaches ClickHouse — it
//! executes the stored SQL as `presentation_ro` and returns untyped JSON rows.
//!
//! Phase-A scope: `/run` binds named parameters server-side — `{tenant}` always
//! (from context), `{period}` when supplied (#1966). The injected tenant-row
//! filter (#1967) is a separate sub-issue.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, NotSet, QueryFilter, Set};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::AppState;
use super::error::SavedQueryError;
use crate::domain::query_gate::validate_single_select;
use crate::domain::saved_query::{
    CreateSavedQueryRequest, RunResponse, RunSavedQueryRequest, SavedQuery, SavedQuerySummary,
    UpdateSavedQueryRequest,
};
use crate::infra::db::entities::saved_queries;

// ── CRUD ────────────────────────────────────────────────────

pub async fn list_saved_queries(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    let rows = saved_queries::Entity::find()
        .filter(saved_queries::Column::InsightTenantId.eq(ctx.subject_tenant_id()))
        .all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to list saved queries");
            CanonicalError::internal("failed to list saved queries").create()
        })?;

    let items: Vec<SavedQuerySummary> = rows.into_iter().map(model_to_summary).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn get_saved_query(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    let row = find_saved_query(&state, ctx.subject_tenant_id(), id).await?;
    Ok(Json(model_to_saved_query(row)))
}

pub async fn create_saved_query(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<CreateSavedQueryRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    validate_single_select(&req.sql).map_err(invalid_sql)?;

    let id = Uuid::now_v7();
    let model = saved_queries::ActiveModel {
        id: Set(id),
        insight_tenant_id: Set(ctx.subject_tenant_id()),
        name: Set(req.name),
        description: Set(req.description),
        sql: Set(req.sql),
        created_at: NotSet,
        updated_at: NotSet,
    };

    saved_queries::Entity::insert(model)
        .exec(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to create saved query");
            CanonicalError::internal("failed to create saved query").create()
        })?;

    let row = find_saved_query(&state, ctx.subject_tenant_id(), id).await?;
    Ok((StatusCode::CREATED, Json(model_to_saved_query(row))))
}

pub async fn update_saved_query(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSavedQueryRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let existing = find_saved_query(&state, ctx.subject_tenant_id(), id).await?;
    let mut model: saved_queries::ActiveModel = existing.into();

    if let Some(name) = req.name {
        model.name = Set(name);
    }
    // Explicit null clears description; absent field leaves it unchanged.
    if let Some(desc) = req.description {
        model.description = Set(desc);
    }
    if let Some(sql) = req.sql {
        validate_single_select(&sql).map_err(|e| invalid_sql_for(id, e))?;
        model.sql = Set(sql);
    }
    model.updated_at = Set(chrono::Utc::now());

    let updated = model.update(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "failed to update saved query");
        CanonicalError::internal("failed to update saved query").create()
    })?;

    Ok(Json(model_to_saved_query(updated)))
}

pub async fn delete_saved_query(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    let existing = find_saved_query(&state, ctx.subject_tenant_id(), id).await?;

    saved_queries::Entity::delete_by_id(existing.id)
        .exec(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to delete saved query");
            CanonicalError::internal("failed to delete saved query").create()
        })?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Run ─────────────────────────────────────────────────────

pub async fn run_saved_query(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
    body: Option<Json<RunSavedQueryRequest>>,
) -> Result<impl IntoResponse, CanonicalError> {
    let saved = find_saved_query(&state, ctx.subject_tenant_id(), id).await?;

    // Re-validate on run: the gate is the write-side barrier, but stored SQL is
    // gated again here so a run can never reach ClickHouse with anything but a
    // single read (defense in depth alongside the `presentation_ro` grants).
    validate_single_select(&saved.sql).map_err(|e| invalid_sql_for(id, e))?;

    // Named parameters (#1966): `{tenant}` is always bound from context; the
    // optional `period` binds `{period}`. Both go through ClickHouse's
    // server-side parameter interface, so a value can never alter the SQL. The
    // injected tenant-row *filter* (#1967) is a separate concern.
    let period = body.and_then(|Json(b)| b.period);
    let rows = execute_read(
        &state,
        id,
        &saved.sql,
        ctx.subject_tenant_id(),
        period.as_deref(),
    )
    .await?;
    Ok(Json(RunResponse { rows }))
}

/// Execute a single read statement against ClickHouse and parse the
/// `JSONEachRow` stream into untyped rows — the same read path the metric query
/// uses. Binds the `tenant`/`period` named parameters server-side (#1966).
async fn execute_read(
    state: &AppState,
    id: Uuid,
    sql: &str,
    tenant_id: Uuid,
    period: Option<&str>,
) -> Result<Vec<serde_json::Value>, CanonicalError> {
    tracing::debug!(sql = %sql, "executing saved query");

    // `.param` sets `param_<name>` on the request; the SQL references it as
    // `{name:Type}`. `tenant` is always bound (unused by a query that omits it
    // — ClickHouse ignores extra params); `period` only when supplied.
    let mut query = state.ch.query(sql).param("tenant", tenant_id.to_string());
    if let Some(period) = period {
        query = query.param("period", period);
    }

    let mut cursor = query.fetch_bytes("JSONEachRow").map_err(|e| {
        tracing::error!(error = %e, sql = %sql, "ClickHouse query failed");
        classify_run_error(id, &e.to_string())
    })?;

    let raw_bytes = cursor.collect().await.map_err(|e| {
        tracing::error!(error = %e, sql = %sql, "ClickHouse fetch failed");
        classify_run_error(id, &e.to_string())
    })?;

    parse_json_each_row(&raw_bytes).map_err(|e| {
        tracing::error!(error = %e, "failed to parse ClickHouse JSON response");
        CanonicalError::internal("failed to parse query results").create()
    })
}

/// Map a ClickHouse run error to a canonical error. A query that references a
/// named parameter left unbound (e.g. `{period}` with no `period` supplied) is
/// caller error (`UNKNOWN_QUERY_PARAMETER`, code 456) — surface it as a 400 so
/// the console can prompt for the missing value rather than a bare 500. The
/// unbound parameter's name is reported when ClickHouse names it, so a query
/// using `{region}` is not mislabeled as a missing `period`.
fn classify_run_error(id: Uuid, message: &str) -> CanonicalError {
    if message.contains("UNKNOWN_QUERY_PARAMETER") || message.contains("Code: 456") {
        let param = missing_param_name(message);
        let reason = match param {
            Some(name) => {
                format!("named parameter `{name}` is referenced by the query but was not supplied")
            }
            None => "a named parameter referenced by the query was not supplied".to_owned(),
        };
        return SavedQueryError::invalid_argument()
            .with_resource(id.to_string())
            .with_field_violation(param.unwrap_or("params"), reason, "MISSING")
            .create();
    }
    CanonicalError::internal("query execution failed").create()
}

/// Extract the unbound parameter's name from a ClickHouse
/// `UNKNOWN_QUERY_PARAMETER` message. ClickHouse backtick-quotes the name
/// (`Substitution ``period`` is not set`); return the first such token, or
/// `None` when the message does not name it.
fn missing_param_name(message: &str) -> Option<&str> {
    let start = message.find('`')? + 1;
    let len = message[start..].find('`')?;
    Some(&message[start..start + len])
}

/// Parse a `JSONEachRow` byte stream (one JSON object per line) into untyped
/// rows. Empty input yields no rows; blank lines are skipped; a malformed line
/// is an error. Pure so the row-shaping is unit-testable without ClickHouse.
fn parse_json_each_row(raw: &[u8]) -> Result<Vec<serde_json::Value>, serde_json::Error> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    raw.split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice)
        .collect()
}

// ── Helpers ─────────────────────────────────────────────────

async fn find_saved_query(
    state: &AppState,
    tenant_id: Uuid,
    id: Uuid,
) -> Result<saved_queries::Model, CanonicalError> {
    saved_queries::Entity::find_by_id(id)
        .filter(saved_queries::Column::InsightTenantId.eq(tenant_id))
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to find saved query");
            CanonicalError::internal("failed to find saved query").create()
        })?
        .ok_or_else(|| {
            SavedQueryError::not_found("saved query not found")
                .with_resource(id.to_string())
                .create()
        })
}

fn invalid_sql(reason: String) -> CanonicalError {
    SavedQueryError::invalid_argument()
        .with_field_violation("sql", reason, "INVALID")
        .create()
}

fn invalid_sql_for(id: Uuid, reason: String) -> CanonicalError {
    SavedQueryError::invalid_argument()
        .with_resource(id.to_string())
        .with_field_violation("sql", reason, "INVALID")
        .create()
}

fn model_to_saved_query(m: saved_queries::Model) -> SavedQuery {
    SavedQuery {
        id: m.id,
        insight_tenant_id: m.insight_tenant_id,
        name: m.name,
        description: m.description,
        sql: m.sql,
        created_at: m.created_at.naive_utc(),
        updated_at: m.updated_at.naive_utc(),
    }
}

fn model_to_summary(m: saved_queries::Model) -> SavedQuerySummary {
    SavedQuerySummary {
        id: m.id,
        name: m.name,
        description: m.description,
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_run_error, missing_param_name, parse_json_each_row};
    use serde_json::json;
    use toolkit_canonical_errors::Problem;
    use uuid::Uuid;

    type R = Result<(), Box<dyn std::error::Error>>;

    fn problem_of(
        err: toolkit_canonical_errors::CanonicalError,
    ) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(Problem::from(err))
    }

    /// A ClickHouse unbound-parameter error (code 456) is caller error → 400, not
    /// a bare 500 (#1966). Both the numeric code and the symbolic name the server
    /// includes are matched.
    #[test]
    fn missing_named_param_is_a_400() -> R {
        for msg in [
            "bad response: Code: 456. DB::Exception: Substitution `period` is not set (UNKNOWN_QUERY_PARAMETER)",
            "Code: 456. DB::Exception: Substitution `period` is not set",
            "DB::Exception: ... (UNKNOWN_QUERY_PARAMETER)",
        ] {
            let p = problem_of(classify_run_error(Uuid::now_v7(), msg))?;
            assert_eq!(p["status"], 400, "should be a 400: {msg:?}");
        }
        Ok(())
    }

    /// The 400 names the parameter ClickHouse reports as unbound, so a query
    /// using `{region}` is not mislabeled as a missing `period`; when the message
    /// does not name it, the violation falls back to a generic `params` field.
    #[test]
    fn missing_param_400_reports_the_named_parameter() -> R {
        for (msg, field) in [
            (
                "Code: 456. DB::Exception: Substitution `period` is not set",
                "period",
            ),
            (
                "Code: 456. DB::Exception: Substitution `region` is not set",
                "region",
            ),
            ("DB::Exception: ... (UNKNOWN_QUERY_PARAMETER)", "params"),
        ] {
            let p = problem_of(classify_run_error(Uuid::now_v7(), msg))?;
            assert_eq!(
                p["context"]["field_violations"][0]["field"], field,
                "wrong field for {msg:?}"
            );
        }
        Ok(())
    }

    /// The name extractor returns the first backtick-quoted token, or `None`.
    #[test]
    fn missing_param_name_extracts_backtick_token() {
        assert_eq!(
            missing_param_name("Substitution `period` is not set"),
            Some("period")
        );
        assert_eq!(missing_param_name("no backticks here"), None);
        assert_eq!(missing_param_name("unterminated `oops"), None);
    }

    /// Any other ClickHouse failure stays an opaque 500 — we do not leak engine
    /// internals or misclassify a genuine server fault as caller error.
    #[test]
    fn other_ch_errors_stay_500() -> R {
        for msg in [
            "bad response: Code: 60. DB::Exception: Unknown table",
            "network error: connection refused",
        ] {
            let p = problem_of(classify_run_error(Uuid::now_v7(), msg))?;
            assert_eq!(p["status"], 500, "should be a 500: {msg:?}");
        }
        Ok(())
    }

    #[test]
    fn empty_input_yields_no_rows() -> R {
        assert_eq!(parse_json_each_row(b"")?, Vec::<serde_json::Value>::new());
        Ok(())
    }

    #[test]
    fn parses_one_row_per_line_and_skips_blanks() -> R {
        let raw = b"{\"a\":1}\n{\"a\":2}\n";
        assert_eq!(
            parse_json_each_row(raw)?,
            vec![json!({"a": 1}), json!({"a": 2})]
        );
        Ok(())
    }

    #[test]
    fn single_row_without_trailing_newline() -> R {
        assert_eq!(
            parse_json_each_row(b"{\"x\":\"y\"}")?,
            vec![json!({"x": "y"})]
        );
        Ok(())
    }

    #[test]
    fn malformed_line_is_an_error() {
        assert!(parse_json_each_row(b"{not json}").is_err());
    }
}
