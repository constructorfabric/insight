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
use crate::infra::db::ops_repo::{self, Operation, PERSON_ATTRIBUTES_RECONCILE_OP};

const LIST_DEFAULT_LIMIT: u64 = 50;
const LIST_MAX_LIMIT: u64 = 500;

#[derive(Debug, Serialize, ToSchema)]
pub struct AttributeReconcileOperationResponse {
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
impl toolkit::api::api_dto::ResponseApiDto for AttributeReconcileOperationResponse {}

impl From<Operation> for AttributeReconcileOperationResponse {
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

#[derive(Debug, Serialize, ToSchema)]
pub struct AttributeReconcileListResponse {
    pub items: Vec<AttributeReconcileOperationResponse>,
    pub next_cursor: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for AttributeReconcileListResponse {}

pub async fn get_attribute_reconcile(
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
        .filter(|o| o.operation_type == PERSON_ATTRIBUTES_RECONCILE_OP)
        .ok_or_else(|| {
            PersonAttributeError::not_found("operation not found")
                .with_resource(id.to_string())
                .create()
        })?;
    Ok(Json(AttributeReconcileOperationResponse::from(op)))
}

pub async fn list_attribute_reconcile(
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
        Some(PERSON_ATTRIBUTES_RECONCILE_OP),
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
        .map(AttributeReconcileOperationResponse::from)
        .collect();
    Ok(Json(AttributeReconcileListResponse {
        items,
        next_cursor: None,
    }))
}
