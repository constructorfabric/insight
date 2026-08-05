//! Persons-sync operations journal — read-only HTTP surface.
//!
//! The sync itself is CLI-only (`identity-resolution sync`, run by the Helm
//! `CronJob` or a manual Job — see `crate::sync_runner`); like the
//! persons-seed after #1690, there is no HTTP trigger. The GETs are the
//! observability window over the `operations` rows the CLI runs write:
//! status, summary (`rows` / `max_id` / `max_created_at` / `synced_at` — the
//! resolution watermark), error per run.
//!
//! Own DTO types rather than reusing the seed's: same wire conventions
//! (parsed JSON `request`/`summary`, ISO-8601 timestamps, nulls emitted) but
//! an independent OpenAPI schema, free to grow sync-specific fields.
//!
//! Admin-gated like the seed journal: the caller is the gateway-JWT subject
//! and must hold an active `admin` role in the tenant.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, Query};
use axum::response::IntoResponse;
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::error::PersonsSyncError;
use super::gate::require_admin;
use super::seed::ListParams;
use crate::infra::db::ops_repo::{self, Operation, PERSONS_SYNC_OP};

/// Default page size / cap for the list endpoint (same as the seed journal).
const LIST_DEFAULT_LIMIT: u64 = 50;
const LIST_MAX_LIMIT: u64 = 500;

/// One operation's status. Wire shape matches the seed journal's:
/// `request` and `summary` surfaced as parsed JSON, ISO-8601 timestamps,
/// null fields emitted.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonsSyncOperationResponse {
    pub operation_id: Uuid,
    pub operation_type: String,
    pub status: String,
    pub insight_tenant_id: Uuid,
    pub author_person_id: Uuid,
    #[schema(value_type = Option<Object>)]
    pub request: Option<serde_json::Value>,
    /// On completion: the [`SyncSummary`] — rows copied, `max_id` /
    /// `max_created_at` watermarks, `synced_at`.
    ///
    /// [`SyncSummary`]: crate::domain::sync_service::SyncSummary
    #[schema(value_type = Option<Object>)]
    pub summary: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonsSyncOperationResponse {}

impl From<Operation> for PersonsSyncOperationResponse {
    fn from(op: Operation) -> Self {
        Self {
            operation_id: op.operation_id,
            operation_type: op.operation_type,
            status: op.status.as_db().to_owned(),
            insight_tenant_id: op.insight_tenant_id,
            author_person_id: op.author_person_id,
            request: super::seed::parse_or_null(op.request_json.as_deref()),
            summary: super::seed::parse_or_null(op.summary_json.as_deref()),
            error_message: op.error_message,
            started_at: super::seed::fmt_ts(op.started_at),
            completed_at: op.completed_at.map(super::seed::fmt_ts),
        }
    }
}

/// List response wrapper (typed for OpenAPI). `next_cursor` is declared but
/// always `null` — same non-paginating contract as the seed journal.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonsSyncListResponse {
    pub items: Vec<PersonsSyncOperationResponse>,
    pub next_cursor: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonsSyncListResponse {}

/// `GET /v1/persons-sync/{id}` — poll one operation.
pub async fn get_persons_sync(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    require_admin(&state.db, &ctx).await?;
    let op = ops_repo::get_by_id(&state.db, tenant, id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "get operation failed");
            CanonicalError::internal("failed to read operation").create()
        })?
        .filter(|o| o.operation_type == PERSONS_SYNC_OP)
        .ok_or_else(|| {
            PersonsSyncError::not_found("operation not found")
                .with_resource(id.to_string())
                .create()
        })?;
    Ok(Json(PersonsSyncOperationResponse::from(op)))
}

/// `GET /v1/persons-sync` — list persons-sync operations. Optional `?status=`
/// (unknown values ignored) and `?limit=` (default 50, capped 500), same
/// semantics as the seed journal list.
pub async fn list_persons_sync(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    require_admin(&state.db, &ctx).await?;
    let status = super::seed::status_filter(params.status.as_deref());
    let limit = params.limit.map_or(LIST_DEFAULT_LIMIT, |l| {
        u64::try_from(l).unwrap_or(1).clamp(1, LIST_MAX_LIMIT)
    });
    let ops = ops_repo::list(&state.db, tenant, Some(PERSONS_SYNC_OP), status, limit)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "list operations failed");
            CanonicalError::internal("failed to list operations").create()
        })?;
    let items = ops
        .into_iter()
        .map(PersonsSyncOperationResponse::from)
        .collect();
    Ok(Json(PersonsSyncListResponse {
        items,
        next_cursor: None,
    }))
}
