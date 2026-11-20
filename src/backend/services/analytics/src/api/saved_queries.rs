use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use insight_clickhouse::MissingRelation;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, NotSet, QueryFilter, Set};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::error::SavedQueryError;
use super::{AppState, is_admin_caller};
use crate::domain::not_ready::NotReadyReason;
use crate::domain::query_gate::{admin_only_database, validate_single_select};
use crate::domain::saved_query::{
    CreateSavedQueryRequest, RunResponse, RunSavedQueryRequest, SavedQuery, SavedQuerySummary,
    UpdateSavedQueryRequest,
};
use crate::infra::db::entities::saved_queries;
use crate::infra::not_ready;

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
    headers: HeaderMap,
    Json(req): Json<CreateSavedQueryRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    validate_single_select(&req.sql).map_err(invalid_sql)?;
    authorize_read(&state, &headers, &req.sql).await?;

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
    headers: HeaderMap,
    Json(req): Json<UpdateSavedQueryRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let existing = find_saved_query(&state, ctx.subject_tenant_id(), id).await?;
    let mut model: saved_queries::ActiveModel = existing.into();

    if let Some(name) = req.name {
        model.name = Set(name);
    }
    if let Some(desc) = req.description {
        model.description = Set(desc);
    }
    if let Some(sql) = req.sql {
        validate_single_select(&sql).map_err(|e| invalid_sql_for(id, e))?;
        authorize_read(&state, &headers, &sql).await?;
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

pub async fn run_saved_query(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Option<Json<RunSavedQueryRequest>>,
) -> Result<impl IntoResponse, CanonicalError> {
    let saved = find_saved_query(&state, ctx.subject_tenant_id(), id).await?;

    validate_single_select(&saved.sql).map_err(|e| invalid_sql_for(id, e))?;
    authorize_read(&state, &headers, &saved.sql).await?;

    let period = body.and_then(|Json(b)| b.period);
    let run = execute_read(
        &state,
        id,
        &saved.sql,
        ctx.subject_tenant_id(),
        period.as_deref(),
    )
    .await?;
    Ok(Json(match run {
        QueryRun::Rows(rows) => RunResponse {
            rows,
            not_ready: None,
        },
        QueryRun::NotReady(reason) => RunResponse {
            rows: Vec::new(),
            not_ready: Some(reason),
        },
    }))
}

enum QueryRun {
    Rows(Vec<serde_json::Value>),
    NotReady(NotReadyReason),
}

async fn execute_read(
    state: &AppState,
    id: Uuid,
    sql: &str,
    tenant_id: Uuid,
    period: Option<&str>,
) -> Result<QueryRun, CanonicalError> {
    tracing::debug!(sql = %sql, "executing saved query");

    // `.param` sets `param_<name>` on the request; the SQL references it as
    // `{name:Type}`. `tenant` is always bound (unused by a query that omits it
    // — ClickHouse ignores extra params); `period` only when supplied.
    let mut query = state.ch.query(sql).param("tenant", tenant_id.to_string());
    if let Some(period) = period {
        query = query.param("period", period);
    }

    let mut cursor = match query.fetch_bytes("JSONEachRow") {
        Ok(cursor) => cursor,
        Err(e) => return run_outcome(id, sql, &e.to_string()),
    };

    let raw_bytes = match cursor.collect().await {
        Ok(bytes) => bytes,
        Err(e) => return run_outcome(id, sql, &e.to_string()),
    };

    parse_json_each_row(&raw_bytes)
        .map(QueryRun::Rows)
        .map_err(|e| {
            tracing::error!(error = %e, "failed to parse ClickHouse JSON response");
            CanonicalError::internal("failed to parse query results").create()
        })
}

fn run_outcome(id: Uuid, sql: &str, message: &str) -> Result<QueryRun, CanonicalError> {
    if MissingRelation::from_message(message).is_some() {
        not_ready::warn_throttled(message, "saved-query:run");
        return Ok(QueryRun::NotReady(NotReadyReason::SourceRelationMissing));
    }

    tracing::error!(error = %message, sql = %sql, "ClickHouse saved-query run failed");
    Err(classify_run_error(id, message))
}

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

fn parse_json_each_row(raw: &[u8]) -> Result<Vec<serde_json::Value>, serde_json::Error> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    raw.split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice)
        .collect()
}

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

async fn authorize_read(
    state: &AppState,
    headers: &HeaderMap,
    sql: &str,
) -> Result<(), CanonicalError> {
    let Some(database) = admin_only_database(sql) else {
        return Ok(());
    };

    if is_admin_caller(state, headers).await? {
        return Ok(());
    }

    Err(SavedQueryError::permission_denied()
        .with_reason(format!("admin role required to read `{database}`"))
        .create())
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
    use super::{
        QueryRun, classify_run_error, missing_param_name, parse_json_each_row, run_outcome,
    };
    use serde_json::json;
    use toolkit_canonical_errors::Problem;
    use uuid::Uuid;

    type R = Result<(), Box<dyn std::error::Error>>;

    fn problem_of(
        err: toolkit_canonical_errors::CanonicalError,
    ) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(Problem::from(err))
    }

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

    #[test]
    fn missing_param_name_extracts_backtick_token() {
        assert_eq!(
            missing_param_name("Substitution `period` is not set"),
            Some("period")
        );
        assert_eq!(missing_param_name("no backticks here"), None);
        assert_eq!(missing_param_name("unterminated `oops"), None);
    }

    #[test]
    fn other_ch_errors_stay_500() -> R {
        for msg in [
            "Code: 47. DB::Exception: Unknown expression identifier (UNKNOWN_IDENTIFIER)",
            "network error: connection refused",
        ] {
            let p = problem_of(classify_run_error(Uuid::now_v7(), msg))?;
            assert_eq!(p["status"], 500, "should be a 500: {msg:?}");
        }
        Ok(())
    }

    #[test]
    fn an_absent_relation_runs_to_an_empty_answer() -> R {
        for msg in [
            "bad response: Code: 60. DB::Exception: Table insight.x does not exist.",
            "Code: 81. DB::Exception: Database insight does not exist. (UNKNOWN_DATABASE)",
        ] {
            let run = run_outcome(Uuid::now_v7(), "SELECT 1", msg);
            assert!(
                matches!(run, Ok(QueryRun::NotReady(_))),
                "should answer not-ready: {msg:?}"
            );
        }

        let failed = run_outcome(
            Uuid::now_v7(),
            "SELECT 1",
            "Code: 47. DB::Exception: Unknown expression identifier (UNKNOWN_IDENTIFIER)",
        );
        let Err(error) = failed else {
            panic!("a column the relation does not have must fail");
        };
        assert_eq!(problem_of(error)?["status"], 500);
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
