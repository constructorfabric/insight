//! Roles catalogue HTTP surface — CRUD over the global `roles` table.
//!
//! Admin-gated. Hard-DELETE with an atomic in-use guard (a role with active
//! `person_roles` assignments cannot be deleted). Ported 1:1 from the .NET
//! `RolesEndpoints` (ADR-0013). Note: the .NET "role in use" case is a 422
//! `role_in_use`; gears canonical errors have no 422, so it is surfaced as a
//! `failed_precondition` (400) — a documented status-code divergence, same
//! spirit as the `gts://` vs `urn:` error-type divergence.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::http::header::LOCATION;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::canonical_json::CanonicalJson;
use super::error::RoleError;
use super::gate::require_admin;
use crate::infra::db::roles_repo::{self, Role};

const MAX_ROLE_NAME_LEN: usize = 64; // VARCHAR(64)

/// Body of `POST /v1/roles`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRoleRequest {
    pub name: String,
}
impl toolkit::api::api_dto::RequestApiDto for CreateRoleRequest {}

/// One role in the catalogue.
#[derive(Debug, Serialize, ToSchema)]
pub struct RoleResponse {
    pub role_id: Uuid,
    pub name: String,
}
impl toolkit::api::api_dto::ResponseApiDto for RoleResponse {}

impl From<Role> for RoleResponse {
    fn from(r: Role) -> Self {
        Self {
            role_id: r.role_id,
            name: r.name,
        }
    }
}

/// List wrapper.
#[derive(Debug, Serialize, ToSchema)]
pub struct RoleListResponse {
    pub items: Vec<RoleResponse>,
}
impl toolkit::api::api_dto::ResponseApiDto for RoleListResponse {}

/// `POST /v1/roles` — create a role (admin only).
pub async fn create_role(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<CreateRoleRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let author = require_admin(&state.db, &ctx).await?;

    // Trim before validating / storing / duplicate-checking so whitespace-only
    // variants (" Admin " vs "Admin") don't accumulate as near-duplicates in the
    // global catalogue. Stricter than the .NET service (which stored verbatim) —
    // a deliberate data-hygiene deviation.
    let name = req.name.trim().to_owned();
    if !role_name_valid(&name) {
        return Err(RoleError::invalid_argument()
            .with_field_violation(
                "name",
                "name must be non-empty and at most 64 characters",
                "INVALID",
            )
            .create());
    }

    // Pre-check duplicate name → friendly 409 (the UNIQUE(name) index would
    // otherwise surface as an opaque 500). Parity with .NET.
    if roles_repo::get_by_name(&state.db, &name)
        .await
        .map_err(read_err)?
        .is_some()
    {
        return Err(already_exists(&name));
    }

    let role_id = Uuid::now_v7();
    if let Err(e) = roles_repo::insert_role(&state.db, role_id, &name).await {
        // Lost the UNIQUE(name) race with a concurrent create between the
        // pre-check and here → 409, not an opaque 500. Re-read to confirm it was
        // a duplicate rather than a real failure.
        if roles_repo::get_by_name(&state.db, &name)
            .await
            .map_err(read_err)?
            .is_some()
        {
            return Err(already_exists(&name));
        }
        tracing::error!(error = %e, "insert role failed");
        return Err(CanonicalError::internal("failed to create role").create());
    }
    tracing::info!(%role_id, %name, author_person_id = %author, "roles.create");

    let location = format!("/v1/roles/{role_id}");
    let body = RoleResponse { role_id, name };
    Ok((StatusCode::CREATED, [(LOCATION, location)], Json(body)))
}

/// `GET /v1/roles` — list all roles (admin only).
pub async fn list_roles(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state.db, &ctx).await?;

    let roles = roles_repo::list_all(&state.db).await.map_err(read_err)?;
    let items = roles.into_iter().map(RoleResponse::from).collect();
    Ok(Json(RoleListResponse { items }))
}

/// `DELETE /v1/roles/{id}` — hard-delete a role (admin only); refuses with a
/// precondition error if the role still has active assignments.
pub async fn delete_role(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    let author = require_admin(&state.db, &ctx).await?;

    let existing = roles_repo::get_by_id(&state.db, id)
        .await
        .map_err(read_err)?
        .ok_or_else(|| not_found(id))?;

    let deleted = roles_repo::try_delete_if_unused(&state.db, id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "delete role failed");
            CanonicalError::internal("failed to delete role").create()
        })?;
    if deleted == 1 {
        tracing::info!(role_id = %id, name = %existing.name, author_person_id = %author, "roles.delete");
        return Ok(StatusCode::NO_CONTENT);
    }

    // 0 rows: either the role vanished between the fetch and the delete (404),
    // or the in-use guard fired (a second read + count tells which).
    if roles_repo::get_by_id(&state.db, id)
        .await
        .map_err(read_err)?
        .is_none()
    {
        return Err(not_found(id));
    }
    let live = roles_repo::count_active_assignments_any_tenant(&state.db, id)
        .await
        .map_err(read_err)?;
    Err(RoleError::failed_precondition()
        .with_precondition_violation(
            id.to_string(),
            format!("role has {live} active assignment(s); revoke them before deletion"),
            "role_in_use",
        )
        .create())
}

fn not_found(id: Uuid) -> CanonicalError {
    RoleError::not_found("role not found")
        .with_resource(id.to_string())
        .create()
}

fn already_exists(name: &str) -> CanonicalError {
    RoleError::already_exists(format!("role name '{name}' already exists"))
        .with_resource(name.to_owned())
        .create()
}

// Takes the error by value so it can be used directly as `.map_err(read_err)`.
#[allow(clippy::needless_pass_by_value)]
fn read_err(e: anyhow::Error) -> CanonicalError {
    tracing::error!(error = %e, "roles query failed");
    CanonicalError::internal("failed to read roles").create()
}

/// `name` must be non-empty (ignoring surrounding whitespace) and at most 64
/// chars — mirrors the .NET `NotEmpty` + `MaximumLength(64)` validator.
fn role_name_valid(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= MAX_ROLE_NAME_LEN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_name_validation() {
        assert!(role_name_valid("admin"));
        assert!(
            role_name_valid(&"a".repeat(MAX_ROLE_NAME_LEN)),
            "64 chars ok"
        );
        assert!(!role_name_valid(""), "empty");
        assert!(!role_name_valid("   "), "whitespace-only");
        assert!(
            !role_name_valid(&"a".repeat(MAX_ROLE_NAME_LEN + 1)),
            "65 chars too long"
        );
    }
}
