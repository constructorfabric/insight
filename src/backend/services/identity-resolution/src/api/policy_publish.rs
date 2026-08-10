use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, Query};
use axum::http::{StatusCode, header::LOCATION};
use axum::response::IntoResponse;
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::error::{AccessError, PersonAttributeError};
use super::gate::require_admin;
use super::seed::ListParams;
use crate::infra::db::ops_repo::{self, Operation, PERSON_ATTRIBUTES_POLICY_PUBLISH_OP};
use crate::infra::db::{self};
use crate::publish_policy_runner::{self, PublishTrigger};

const LIST_DEFAULT_LIMIT: u64 = 50;
const LIST_MAX_LIMIT: u64 = 500;

#[derive(Debug, Serialize, ToSchema)]
pub struct PolicyPublishOperationResponse {
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

impl PolicyPublishOperationResponse {
    fn queued(
        operation_id: Uuid,
        tenant: Uuid,
        author: Uuid,
        trigger: PublishTrigger,
        started_at: sea_orm::prelude::DateTime,
    ) -> Self {
        Self {
            operation_id,
            operation_type: PERSON_ATTRIBUTES_POLICY_PUBLISH_OP.to_owned(),
            status: "queued".to_owned(),
            insight_tenant_id: tenant,
            author_person_id: author,
            request: super::seed::parse_or_null(Some(trigger.request_json())),
            summary: None,
            error_message: None,
            started_at: super::seed::fmt_ts(started_at),
            completed_at: None,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PolicyPublishListResponse {
    pub items: Vec<PolicyPublishOperationResponse>,
    pub next_cursor: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for PolicyPublishListResponse {}

fn publish_in_progress() -> CanonicalError {
    PersonAttributeError::aborted("a policy publish is already running")
        .with_reason("PUBLISH_IN_PROGRESS")
        .create()
}

fn publish_unavailable() -> CanonicalError {
    CanonicalError::internal("failed to start policy publish").create()
}

fn tenant_mismatch() -> CanonicalError {
    AccessError::failed_precondition()
        .with_precondition_violation(
            "tenant",
            "the caller's tenant is not the tenant this deployment journals policy publishes under",
            "tenant_mismatch",
        )
        .create()
}

pub async fn create_policy_publish(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    let author = require_admin(&state.db, &ctx).await?;

    let tenant = publish_policy_runner::resolve_journal_tenant(&state.db, &state.config)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "policy-publish: journal tenant unresolved");
            publish_unavailable()
        })?;
    if tenant != ctx.subject_tenant_id() {
        return Err(tenant_mismatch());
    }

    let Some(lock) = db::PolicyPublishLockGuard::try_acquire(&state.config.database_url)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "policy-publish: lock acquire failed");
            publish_unavailable()
        })?
    else {
        return Err(publish_in_progress());
    };

    let started_at = chrono::Utc::now().naive_utc();
    let operation_id =
        publish_policy_runner::enqueue_run(&state.db, tenant, author, PublishTrigger::Http)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "policy-publish: enqueue failed");
                publish_unavailable()
            })?;

    tokio::spawn(publish_policy_runner::run_detached(
        state.db.clone(),
        state.config.clone(),
        state.cancel.clone(),
        tenant,
        operation_id,
        lock,
    ));

    let body = PolicyPublishOperationResponse::queued(
        operation_id,
        tenant,
        author,
        PublishTrigger::Http,
        started_at,
    );
    let location = format!("/v1/person-attributes-policy-publish/{operation_id}");
    Ok((StatusCode::ACCEPTED, [(LOCATION, location)], Json(body)))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_concurrent_publish_is_refused_as_409() {
        assert_eq!(publish_in_progress().status_code(), StatusCode::CONFLICT);
    }

    #[test]
    fn a_caller_outside_the_journal_tenant_is_refused_as_400() {
        assert_eq!(tenant_mismatch().status_code(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn the_accepted_body_reports_queued_with_no_outcome_yet() -> anyhow::Result<()> {
        let started =
            sea_orm::prelude::DateTime::parse_from_str("2026-08-06 12:00:00", "%Y-%m-%d %H:%M:%S")?;
        let id = Uuid::from_u128(9);

        let body = PolicyPublishOperationResponse::queued(
            id,
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            PublishTrigger::Http,
            started,
        );

        assert_eq!(body.operation_id, id);
        assert_eq!(body.status, "queued");
        assert_eq!(body.summary, None);
        assert_eq!(body.completed_at, None);
        assert_eq!(body.error_message, None);
        Ok(())
    }
}
