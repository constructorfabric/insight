//! Policy-publish operations journal — read-only HTTP surface.
//!
//! The publish itself is CLI-only (`identity-resolution publish-policy`, run
//! by the Helm `CronJob` or a manual Job — see
//! `crate::publish_policy_runner`); these GETs are the observability window
//! over its `operations` rows: status, summary (rows, checksum, whether the
//! run skipped an unchanged snapshot), error per run. Same wire conventions
//! and admin gate as the seed and sync journals.

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
use super::error::PersonAttributeError;
use super::gate::require_admin;
use super::seed::ListParams;
use crate::infra::db::ops_repo::{self, Operation, PERSON_ATTRIBUTES_POLICY_PUBLISH_OP};

const LIST_DEFAULT_LIMIT: u64 = 50;
const LIST_MAX_LIMIT: u64 = 500;

/// One reconcile operation's status.
#[derive(Debug, Serialize, ToSchema)]
pub struct PolicyPublishOperationResponse {
    pub operation_id: Uuid,
    pub operation_type: String,
    pub status: String,
    pub insight_tenant_id: Uuid,
    pub author_person_id: Uuid,
    #[schema(value_type = Option<Object>)]
    pub request: Option<serde_json::Value>,
    /// On completion: the [`PublishSummary`] — published row count, content
    /// checksum, and whether the run skipped an already-current snapshot.
    ///
    /// [`PublishSummary`]: crate::publish_policy_runner::PublishSummary
    #[schema(value_type = Option<Object>)]
    pub summary: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for PolicyPublishOperationResponse {}

impl From<Operation> for PolicyPublishOperationResponse {
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

/// List response wrapper (typed for OpenAPI); `next_cursor` is declared but
/// always `null`, same non-paginating contract as the other journals.
#[derive(Debug, Serialize, ToSchema)]
pub struct PolicyPublishListResponse {
    pub items: Vec<PolicyPublishOperationResponse>,
    pub next_cursor: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for PolicyPublishListResponse {}

/// `GET /v1/person-attributes-policy-publish/{id}` — poll one operation.
pub async fn get_policy_publish(
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
        .filter(|o| o.operation_type == PERSON_ATTRIBUTES_POLICY_PUBLISH_OP)
        .ok_or_else(|| {
            PersonAttributeError::not_found("operation not found")
                .with_resource(id.to_string())
                .create()
        })?;
    Ok(Json(PolicyPublishOperationResponse::from(op)))
}

/// `GET /v1/person-attributes-policy-publish` — list reconcile operations.
/// Optional `?status=` (unknown values ignored) and `?limit=` (default 50,
/// capped 500), same semantics as the seed and sync journals.
pub async fn list_policy_publish(
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
    let ops = ops_repo::list(
        &state.db,
        tenant,
        Some(PERSON_ATTRIBUTES_POLICY_PUBLISH_OP),
        status,
        limit,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "list operations failed");
        CanonicalError::internal("failed to list operations").create()
    })?;
    let items = ops
        .into_iter()
        .map(PolicyPublishOperationResponse::from)
        .collect();
    Ok(Json(PolicyPublishListResponse {
        items,
        next_cursor: None,
    }))
}
