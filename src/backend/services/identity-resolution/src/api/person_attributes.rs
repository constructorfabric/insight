//! Person-attribute registry — admin HTTP surface.
//!
//! Definitions are discovered by the `reconcile-attributes` CLI run (see
//! `crate::attribute_reconcile_runner`); this surface reads them and revises
//! their policy. A policy revision is append-only: `PUT …/policy` writes the
//! next revision, never mutates one, and carries the caller as its actor.
//! Concurrency is optimistic — the request names the revision it saw, and a
//! stale value is a 409 `aborted` (the canonical model has no 422; see the
//! divergence note in `super::error`).
//!
//! Tenant scope: definitions store the RAW warehouse tenant string; the
//! caller's gateway-JWT tenant UUID is matched against it in canonical
//! string form.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::error::PersonAttributeError;
use super::gate::require_admin;
use crate::infra::db::person_attributes_repo::{
    self, DefinitionWithPolicy, PolicyInput, ValueMode,
};

/// One attribute definition with its current policy.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonAttributeResponse {
    pub id: Uuid,
    pub source_type: String,
    pub source_instance: String,
    pub source_field_id: String,
    pub first_observed_at: String,
    pub last_observed_at: String,
    pub policy: PolicyResponse,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonAttributeResponse {}

/// The current policy revision of one definition.
#[derive(Debug, Serialize, ToSchema)]
pub struct PolicyResponse {
    pub revision: i32,
    pub label_override: Option<String>,
    pub sensitivity_class: Option<String>,
    pub grouping_enabled: bool,
    pub comparison_enabled: bool,
    pub value_mode: String,
    pub retired: bool,
    pub actor_person_id: Uuid,
    pub reason: String,
}

/// List response wrapper (typed for OpenAPI).
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonAttributeListResponse {
    pub items: Vec<PersonAttributeResponse>,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonAttributeListResponse {}

/// Declared value mode of an attribute, as an OpenAPI enum.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValueModeDto {
    Single,
    Multi,
}

impl From<ValueModeDto> for ValueMode {
    fn from(v: ValueModeDto) -> Self {
        match v {
            ValueModeDto::Single => Self::Single,
            ValueModeDto::Multi => Self::Multi,
        }
    }
}

/// Full policy body for the next revision. `expected_revision` is the
/// revision the caller read; a stale value yields 409.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PolicyUpdateRequest {
    pub expected_revision: i32,
    pub label_override: Option<String>,
    pub sensitivity_class: Option<String>,
    pub grouping_enabled: bool,
    pub comparison_enabled: bool,
    pub value_mode: ValueModeDto,
    pub retired: bool,
    pub reason: String,
}
impl toolkit::api::api_dto::RequestApiDto for PolicyUpdateRequest {}

fn to_policy_input(body: PolicyUpdateRequest) -> PolicyInput {
    PolicyInput {
        label_override: body.label_override,
        sensitivity_class: body.sensitivity_class,
        grouping_enabled: body.grouping_enabled,
        comparison_enabled: body.comparison_enabled,
        value_mode: body.value_mode.into(),
        retired: body.retired,
        reason: body.reason,
    }
}

fn to_response(d: DefinitionWithPolicy) -> PersonAttributeResponse {
    PersonAttributeResponse {
        id: d.id,
        source_type: d.key.insight_source_type,
        source_instance: d.key.insight_source_id,
        source_field_id: d.key.source_field_id,
        first_observed_at: d.first_observed_at,
        last_observed_at: d.last_observed_at,
        policy: PolicyResponse {
            revision: d.policy.revision,
            label_override: d.policy.label_override,
            sensitivity_class: d.policy.sensitivity_class,
            grouping_enabled: d.policy.grouping_enabled,
            comparison_enabled: d.policy.comparison_enabled,
            value_mode: d.policy.value_mode.as_db().to_owned(),
            retired: d.policy.retired,
            actor_person_id: d.policy.actor_person_id,
            reason: d.policy.reason,
        },
    }
}

/// `GET /v1/person-attributes` — list the tenant's discovered attribute
/// definitions with their current policy.
pub async fn list_person_attributes(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state.db, &ctx).await?;

    let tenant = ctx.subject_tenant_id().to_string();
    let items = person_attributes_repo::list_definitions(&state.db, &tenant)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "list person attributes failed");
            CanonicalError::internal("failed to list person attributes").create()
        })?
        .into_iter()
        .map(to_response)
        .collect();
    Ok(Json(PersonAttributeListResponse { items }))
}

/// `GET /v1/person-attributes/{id}` — one definition with its current policy.
pub async fn get_person_attribute(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state.db, &ctx).await?;

    let tenant = ctx.subject_tenant_id().to_string();
    let definition = person_attributes_repo::get_definition(&state.db, &tenant, id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "get person attribute failed");
            CanonicalError::internal("failed to read person attribute").create()
        })?
        .ok_or_else(|| {
            PersonAttributeError::not_found("person attribute not found")
                .with_resource(id.to_string())
                .create()
        })?;
    Ok(Json(to_response(definition)))
}

/// `PUT /v1/person-attributes/{id}/policy` — append the next policy revision.
/// The 200 body re-reads the current state, which a concurrent append may
/// already have advanced past the revision this call wrote — acceptable
/// under optimistic concurrency (the response always shows current truth).
pub async fn put_person_attribute_policy(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<PolicyUpdateRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state.db, &ctx).await?;

    if body.expected_revision < 1 {
        return Err(PersonAttributeError::invalid_argument()
            .with_field_violation(
                "expected_revision",
                "expected_revision must be >= 1",
                "RANGE",
            )
            .create());
    }

    let tenant = ctx.subject_tenant_id().to_string();
    let expected_revision = body.expected_revision;
    let appended = person_attributes_repo::append_policy_revision(
        &state.db,
        &tenant,
        id,
        expected_revision,
        &to_policy_input(body),
        ctx.subject_id(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "append policy revision failed");
        CanonicalError::internal("failed to update person attribute policy").create()
    })?;
    if !appended {
        return Err(PersonAttributeError::aborted(
            "expected_revision is stale or the person attribute does not exist",
        )
        .with_reason("STALE_REVISION")
        .create());
    }

    let updated = person_attributes_repo::get_definition(&state.db, &tenant, id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "re-read after policy update failed");
            CanonicalError::internal("failed to read person attribute").create()
        })?
        .ok_or_else(|| {
            PersonAttributeError::not_found("person attribute not found")
                .with_resource(id.to_string())
                .create()
        })?;
    Ok(Json(to_response(updated)))
}
