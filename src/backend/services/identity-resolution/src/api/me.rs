//! Caller self-description — `GET /v1/me`.
//!
//! Answers "who does the gateway say I am, and which identity roles do I
//! hold right now?" from the same `person_roles` rows the admin gate reads.
//! Deliberately NOT admin-gated: an empty `roles` list IS the "not an admin"
//! answer, so the SPA can gate its admin surfaces without probing for a 403.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::response::IntoResponse;
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::gate::require_caller;
use crate::config::VisibilityPolicy;
use crate::infra::db::roles_repo::{self, Role};

/// One active role assignment of the caller.
#[derive(Debug, Serialize, ToSchema)]
pub struct MeRoleResponse {
    pub role_id: Uuid,
    pub name: String,
}

impl From<Role> for MeRoleResponse {
    fn from(r: Role) -> Self {
        Self {
            role_id: r.role_id,
            name: r.name,
        }
    }
}

/// The caller as the gateway JWT identifies them, with their active roles.
#[derive(Debug, Serialize, ToSchema)]
pub struct MeResponse {
    pub person_id: Uuid,
    pub insight_tenant_id: Uuid,
    pub roles: Vec<MeRoleResponse>,
    pub visibility_policy: VisibilityPolicy,
}
impl toolkit::api::api_dto::ResponseApiDto for MeResponse {}

/// `GET /v1/me` — the caller's identity and active roles (any signed-in user).
pub async fn get_me(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    let caller = require_caller(&ctx)?;
    let tenant = ctx.subject_tenant_id();

    let roles = roles_repo::active_roles_of_person(&state.db, tenant, caller)
        .await
        .map_err(read_err)?;

    Ok(Json(MeResponse {
        person_id: caller,
        insight_tenant_id: tenant,
        roles: roles.into_iter().map(MeRoleResponse::from).collect(),
        visibility_policy: state.config.visibility_policy,
    }))
}

#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn read_err(e: anyhow::Error) -> CanonicalError {
    tracing::error!(error = %e, "caller roles query failed");
    CanonicalError::internal("failed to read caller roles").create()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_is_named_on_the_wire_as_the_config_names_it() -> anyhow::Result<()> {
        for (policy, expected) in [
            (VisibilityPolicy::OrgChart, "org_chart"),
            (VisibilityPolicy::Flat, "flat"),
        ] {
            let body = serde_json::to_value(MeResponse {
                person_id: Uuid::from_u128(1),
                insight_tenant_id: Uuid::from_u128(2),
                roles: Vec::new(),
                visibility_policy: policy,
            })?;

            assert_eq!(body["visibility_policy"], expected);
        }
        Ok(())
    }
}
