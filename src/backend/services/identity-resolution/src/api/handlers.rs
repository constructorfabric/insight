//! Route handlers.
//!
//! `/health` + `/healthz` + `/docs` are provided by the api-gateway host gear,
//! so we define no health handler.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::error::ProfileError;
use crate::gear::AppState;
use crate::infra::db::persons_repo;

/// Body of `POST /v1/profiles`. `value_type = "email"` matches across all
/// sources for the tenant; `value_type = "id"` matches a source-native account
/// id within one source instance (needs `insight_source_type` + `insight_source_id`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolveProfileCommand {
    pub value_type: String,
    pub value: String,
    /// Required when `value_type = "id"` — the source instance to scope to.
    #[serde(default)]
    pub insight_source_type: Option<String>,
    /// Required when `value_type = "id"`.
    #[serde(default)]
    pub insight_source_id: Option<Uuid>,
}

/// Minimal resolve response — person id only. The full profile (attributes,
/// org, ids[]) lands in the next step.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileIdResponse {
    pub person_id: Uuid,
}

// Marker traits the toolkit `OperationBuilder` requires for request/response
// bodies (alongside `ToSchema`). Empty impls, same as the analytics gear.
impl toolkit::api::api_dto::RequestApiDto for ResolveProfileCommand {}
impl toolkit::api::api_dto::ResponseApiDto for ProfileIdResponse {}

/// `POST /v1/profiles` — resolve one identity to a person.
///
/// email-only for now. 0 matches → 404; >1 → 409 (the .NET service used 422
/// `ambiguous_profile`; gears canonical errors have no 422, so we map to
/// `aborted`/409 — flagged for review).
pub async fn resolve_profile(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(cmd): Json<ResolveProfileCommand>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();

    let person_ids = match cmd.value_type.trim() {
        "email" => persons_repo::resolve_person_ids_by_email(&state.db, tenant, &cmd.value)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "resolve by email failed");
                CanonicalError::internal("profile resolution failed").create()
            })?,
        "id" => {
            let source_type = cmd.insight_source_type.as_deref().ok_or_else(|| {
                ProfileError::invalid_argument()
                    .with_field_violation(
                        "insight_source_type",
                        "insight_source_type is required for value_type='id'",
                        "REQUIRED",
                    )
                    .create()
            })?;
            let source_id = cmd.insight_source_id.ok_or_else(|| {
                ProfileError::invalid_argument()
                    .with_field_violation(
                        "insight_source_id",
                        "insight_source_id is required for value_type='id'",
                        "REQUIRED",
                    )
                    .create()
            })?;
            persons_repo::resolve_person_ids_by_source_id(
                &state.db,
                tenant,
                source_type,
                source_id,
                &cmd.value,
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "resolve by source id failed");
                CanonicalError::internal("profile resolution failed").create()
            })?
        }
        _ => {
            return Err(ProfileError::invalid_argument()
                .with_field_violation(
                    "value_type",
                    "value_type must be 'email' or 'id'",
                    "INVALID",
                )
                .create());
        }
    };

    match person_ids.as_slice() {
        [] => Err(ProfileError::not_found("person not found")
            .with_resource(cmd.value)
            .create()),
        [person_id] => Ok(Json(ProfileIdResponse {
            person_id: *person_id,
        })),
        _ => Err(ProfileError::aborted("email resolves to multiple persons")
            .with_reason("AMBIGUOUS_PROFILE")
            .create()),
    }
}
