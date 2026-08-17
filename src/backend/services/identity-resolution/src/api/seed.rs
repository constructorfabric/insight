//! Persons-seed operations journal — read-only HTTP surface.
//!
//! The seed itself is CLI-only (`identity-resolution seed`, run by the Helm
//! `CronJob` or a manual Job — see `crate::seed_runner`); the former
//! `POST /v1/persons-seed` trigger and its in-process queue/worker are gone
//! (#1690). The GETs remain as the observability window over the
//! `operations` rows the CLI runs write: status, summary, error per run.
//!
//! Admin-gated: the caller is the gateway-JWT
//! subject (`SecurityContext::subject_id`, verified by the host authn pipeline —
//! `NGINX_BFF` R1) and must hold an active `admin` role in the tenant.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, Query};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::error::PersonsSeedError;
use super::gate::require_admin;
use crate::infra::db::ops_repo::{self, Operation, OperationStatus, PERSONS_SEED_OP};

/// Default page size / cap for the list endpoint.
const LIST_DEFAULT_LIMIT: u64 = 50;
const LIST_MAX_LIMIT: u64 = 500;

/// One operation's status. `request` and `summary` are surfaced as parsed
/// JSON (not double-encoded strings), the tenant/author ids are included,
/// timestamps are ISO-8601, and null fields are emitted rather than dropped.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonsSeedOperationResponse {
    pub operation_id: Uuid,
    pub operation_type: String,
    pub status: String,
    pub insight_tenant_id: Uuid,
    pub author_person_id: Uuid,
    #[schema(value_type = Option<Object>)]
    pub request: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub summary: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonsSeedOperationResponse {}

impl From<Operation> for PersonsSeedOperationResponse {
    fn from(op: Operation) -> Self {
        Self {
            operation_id: op.operation_id,
            operation_type: op.operation_type,
            status: op.status.as_db().to_owned(),
            insight_tenant_id: op.insight_tenant_id,
            author_person_id: op.author_person_id,
            request: parse_or_null(op.request_json.as_deref()),
            summary: parse_or_null(op.summary_json.as_deref()),
            error_message: op.error_message,
            started_at: super::datetime::fmt_ts(op.started_at),
            completed_at: op.completed_at.map(super::datetime::fmt_ts),
        }
    }
}

/// Surface a stored JSON column as a parsed value (not a double-encoded string);
/// `None` for absent/empty/unparseable.
/// `pub(crate)`: shared with the persons-sync journal (same wire conventions).
pub(crate) fn parse_or_null(json: Option<&str>) -> Option<serde_json::Value> {
    let s = json?;
    if s.is_empty() {
        return None;
    }
    serde_json::from_str(s).ok()
}

/// List response wrapper (typed for OpenAPI).
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonsSeedListResponse {
    pub items: Vec<PersonsSeedOperationResponse>,
    /// The cursor is declared but pagination is not implemented — always
    /// `null`; the route returns every row.
    pub next_cursor: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonsSeedListResponse {}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub status: Option<String>,
    // Signed so a negative `?limit=` clamps to 1 (parity with the sibling list
    // routes) rather than failing query deserialization.
    pub limit: Option<i64>,
}

/// `GET /v1/persons-seed/{id}` — poll one operation.
pub async fn get_persons_seed(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    // Same admin gate as the sibling journal route: the whole persons-seed
    // surface is admin-only.
    require_admin(&state.db, &ctx).await?;
    let op = ops_repo::get_by_id(&state.db, tenant, id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "get operation failed");
            CanonicalError::internal("failed to read operation").create()
        })?
        .filter(|o| o.operation_type == PERSONS_SEED_OP)
        .ok_or_else(|| {
            PersonsSeedError::not_found("operation not found")
                .with_resource(id.to_string())
                .create()
        })?;
    Ok(Json(PersonsSeedOperationResponse::from(op)))
}

/// `GET /v1/persons-seed` — list persons-seed operations. Optional `?status=`
/// (unknown values ignored) and `?limit=` (default 50, capped 500).
pub async fn list_persons_seed(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    require_admin(&state.db, &ctx).await?;
    let status = status_filter(params.status.as_deref());
    let limit = params.limit.map_or(LIST_DEFAULT_LIMIT, |l| {
        u64::try_from(l).unwrap_or(1).clamp(1, LIST_MAX_LIMIT)
    });
    let ops = ops_repo::list(&state.db, tenant, Some(PERSONS_SEED_OP), status, limit)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "list operations failed");
            CanonicalError::internal("failed to list operations").create()
        })?;
    let items = ops
        .into_iter()
        .map(PersonsSeedOperationResponse::from)
        .collect();
    Ok(Json(PersonsSeedListResponse {
        items,
        next_cursor: None,
    }))
}

/// Map the `?status=` query to a filter. An unknown/blank value is ignored
/// (returns all statuses) — not a 400.
pub(crate) fn status_filter(raw: Option<&str>) -> Option<OperationStatus> {
    match raw {
        Some("queued") => Some(OperationStatus::Queued),
        Some("running") => Some(OperationStatus::Running),
        Some("completed") => Some(OperationStatus::Completed),
        Some("failed") => Some(OperationStatus::Failed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_filter_maps_known_and_ignores_unknown() {
        assert_eq!(status_filter(Some("queued")), Some(OperationStatus::Queued));
        assert_eq!(
            status_filter(Some("running")),
            Some(OperationStatus::Running)
        );
        assert_eq!(
            status_filter(Some("completed")),
            Some(OperationStatus::Completed)
        );
        assert_eq!(status_filter(Some("failed")), Some(OperationStatus::Failed));
        assert_eq!(status_filter(Some("bogus")), None);
        assert_eq!(status_filter(Some("")), None);
        assert_eq!(status_filter(None), None);
    }

    #[test]
    fn parse_or_null_parses_and_tolerates_garbage() {
        assert_eq!(
            parse_or_null(Some(r#"{"mode":"link-by-email"}"#)),
            Some(serde_json::json!({"mode": "link-by-email"}))
        );
        assert_eq!(parse_or_null(Some("")), None);
        assert_eq!(parse_or_null(Some("not-json")), None);
        assert_eq!(parse_or_null(None), None);
    }
}
