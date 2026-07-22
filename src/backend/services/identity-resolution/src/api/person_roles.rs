//! Person-roles junction HTTP surface — grant / list / revoke role assignments.
//!
//! Admin-gated; ported 1:1 from the .NET `PersonRolesEndpoints` (ADR-0014).
//! Revoke refuses to remove the tenant's LAST active `admin` assignment
//! (lockout protection). As in the roles domain, the .NET last-admin 422 has no
//! gears canonical equivalent → surfaced as `failed_precondition` (400).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::http::header::LOCATION;
use axum::response::IntoResponse;
use sea_orm::prelude::DateTime;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::canonical_json::CanonicalJson;
use super::error::PersonRoleError;
use super::gate::require_admin;
use crate::infra::db::person_roles_repo::{self, PersonRole};
use crate::infra::db::roles_repo::ADMIN_ROLE_ID;

const LIST_DEFAULT_LIMIT: u64 = 50;
const LIST_MAX_LIMIT: u64 = 500;

/// Body of `POST /v1/person-roles` — grant a role to a person.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePersonRoleRequest {
    pub person_id: Uuid,
    pub role_id: Uuid,
    /// Optional assignment start; defaults to now when omitted.
    #[serde(default)]
    pub valid_from: Option<DateTime>,
    #[serde(default)]
    pub reason: Option<String>,
}
impl toolkit::api::api_dto::RequestApiDto for CreatePersonRoleRequest {}

/// One role assignment.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonRoleResponse {
    pub person_role_id: Uuid,
    pub insight_tenant_id: Uuid,
    pub person_id: Uuid,
    pub role_id: Uuid,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub author_person_id: Uuid,
    pub reason: Option<String>,
    pub created_at: String,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonRoleResponse {}

impl From<PersonRole> for PersonRoleResponse {
    fn from(p: PersonRole) -> Self {
        Self {
            person_role_id: p.person_role_id,
            insight_tenant_id: p.insight_tenant_id,
            person_id: p.person_id,
            role_id: p.role_id,
            valid_from: fmt_ts(p.valid_from),
            valid_to: p.valid_to.map(fmt_ts),
            author_person_id: p.author_person_id,
            reason: p.reason,
            created_at: fmt_ts(p.created_at),
        }
    }
}

/// List wrapper.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonRoleListResponse {
    pub items: Vec<PersonRoleResponse>,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonRoleListResponse {}

/// Optional `DELETE` body carrying a revoke reason.
#[derive(Debug, Deserialize)]
pub struct RevokeReasonRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub person: Option<Uuid>,
    pub role: Option<Uuid>,
    pub active: Option<bool>,
    pub limit: Option<u64>,
}

/// `POST /v1/person-roles` — grant a role (admin only).
pub async fn create_person_role(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<CreatePersonRoleRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let author = require_admin(&state.db, &ctx).await?;

    if !ids_present(req.person_id, req.role_id) {
        return Err(PersonRoleError::invalid_argument()
            .with_field_violation("person_id", "person_id and role_id are required", "INVALID")
            .create());
    }

    let person_role_id = Uuid::now_v7();
    person_roles_repo::insert(
        &state.db,
        person_role_id,
        tenant,
        req.person_id,
        req.role_id,
        req.valid_from,
        author,
        req.reason.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "insert person_role failed");
        CanonicalError::internal("failed to create assignment").create()
    })?;
    tracing::info!(%person_role_id, person_id = %req.person_id, role_id = %req.role_id, author_person_id = %author, "person_roles.create");

    // Read back the inserted row (its DB-assigned valid_from / created_at).
    let created = person_roles_repo::get_by_id(&state.db, tenant, person_role_id)
        .await
        .map_err(read_err)?
        .ok_or_else(|| CanonicalError::internal("assignment vanished after insert").create())?;

    let location = format!("/v1/person-roles/{person_role_id}");
    Ok((
        StatusCode::CREATED,
        [(LOCATION, location)],
        Json(PersonRoleResponse::from(created)),
    ))
}

/// `GET /v1/person-roles` — list assignments (admin only). Filters: `?person=`,
/// `?role=`, `?active=` (default false = all), `?limit=` (default 50, cap 500).
pub async fn list_person_roles(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    require_admin(&state.db, &ctx).await?;

    let limit = params
        .limit
        .unwrap_or(LIST_DEFAULT_LIMIT)
        .clamp(1, LIST_MAX_LIMIT);
    let rows = person_roles_repo::list(
        &state.db,
        tenant,
        params.person,
        params.role,
        params.active.unwrap_or(false),
        limit,
    )
    .await
    .map_err(read_err)?;
    let items = rows.into_iter().map(PersonRoleResponse::from).collect();
    Ok(Json(PersonRoleListResponse { items }))
}

/// `DELETE /v1/person-roles/{id}` — revoke an assignment (admin only); refuses
/// to remove the tenant's last active admin.
pub async fn delete_person_role(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
    body: Option<Json<RevokeReasonRequest>>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let author = require_admin(&state.db, &ctx).await?;
    let reason = body.and_then(|Json(b)| b.reason);

    // Pre-fetch for audit + initial 404. A revoked row (valid_to set) is 404.
    let existing = match person_roles_repo::get_by_id(&state.db, tenant, id)
        .await
        .map_err(read_err)?
    {
        Some(pr) if pr.valid_to.is_none() => pr,
        _ => return Err(not_found(id)),
    };

    let revoked = person_roles_repo::try_soft_delete_protecting_last_admin(
        &state.db,
        id,
        ADMIN_ROLE_ID,
        reason.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "revoke person_role failed");
        CanonicalError::internal("failed to revoke assignment").create()
    })?;
    if revoked == 1 {
        tracing::info!(person_role_id = %id, person_id = %existing.person_id, role_id = %existing.role_id, author_person_id = %author, "person_roles.revoke");
        return Ok(StatusCode::NO_CONTENT);
    }

    // 0 rows: revoked concurrently (404) or the last-admin guard fired.
    let refetched = person_roles_repo::get_by_id(&state.db, tenant, id)
        .await
        .map_err(read_err)?;
    if refetched.is_none_or(|pr| pr.valid_to.is_some()) {
        return Err(not_found(id));
    }
    Err(PersonRoleError::failed_precondition()
        .with_precondition_violation(
            id.to_string(),
            "cannot revoke the last active admin assignment in this tenant",
            "last_admin_protected",
        )
        .create())
}

fn not_found(id: Uuid) -> CanonicalError {
    PersonRoleError::not_found("person_role not found")
        .with_resource(id.to_string())
        .create()
}

// Takes the error by value so it can be used directly as `.map_err(read_err)`.
#[allow(clippy::needless_pass_by_value)]
fn read_err(e: anyhow::Error) -> CanonicalError {
    tracing::error!(error = %e, "person_roles query failed");
    CanonicalError::internal("failed to read assignments").create()
}

/// Format a DB `DateTime` (naive) as ISO-8601 with a `T` separator, matching the
/// .NET `System.Text.Json` `DateTime` output.
fn fmt_ts(dt: DateTime) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}

/// Both ids must be present (non-nil) — mirrors the .NET `NotEmpty` validators
/// on `PersonId` / `RoleId`.
fn ids_present(person_id: Uuid, role_id: Uuid) -> bool {
    !person_id.is_nil() && !role_id.is_nil()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_present_requires_both_non_nil() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert!(ids_present(a, b));
        assert!(!ids_present(Uuid::nil(), b), "nil person");
        assert!(!ids_present(a, Uuid::nil()), "nil role");
        assert!(!ids_present(Uuid::nil(), Uuid::nil()), "both nil");
    }
}
