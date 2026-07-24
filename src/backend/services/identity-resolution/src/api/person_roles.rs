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
const MAX_REASON_LEN: usize = 500; // VARCHAR(500)

/// Body of `POST /v1/person-roles` — grant a role to a person.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePersonRoleRequest {
    pub person_id: Uuid,
    pub role_id: Uuid,
    /// Optional assignment start; defaults to now when omitted. Accepts RFC-3339
    /// (`Z`/offset), zone-less, or date-only, normalised to naive-UTC.
    #[serde(default, deserialize_with = "super::datetime::deserialize_opt")]
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
    // Signed so a negative `?limit=` clamps to 1 (parity with the .NET `int?`
    // clamp) rather than failing query deserialization.
    pub limit: Option<i64>,
}

/// `POST /v1/person-roles` — grant a role (admin only).
pub async fn create_person_role(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<CreatePersonRoleRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let author = require_admin(&state.db, &ctx).await?;

    // Per-field validation, mirroring the .NET `CreatePersonRoleCommandValidator`
    // (`invalid_person_id` / `invalid_role_id` / `invalid_reason`).
    if req.person_id.is_nil() {
        return Err(invalid_field(
            "person_id",
            "person_id is required",
            "invalid_person_id",
        ));
    }
    if req.role_id.is_nil() {
        return Err(invalid_field(
            "role_id",
            "role_id is required",
            "invalid_role_id",
        ));
    }
    if !reason_valid(req.reason.as_deref()) {
        return Err(reason_too_long());
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

    let limit = clamp_limit(params.limit);
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
    if !reason_valid(reason.as_deref()) {
        return Err(reason_too_long());
    }

    // Pre-fetch for audit + initial 404. A revoked row (valid_to set) is 404.
    let existing = match person_roles_repo::get_by_id(&state.db, tenant, id)
        .await
        .map_err(read_err)?
    {
        Some(pr) if pr.valid_to.is_none() => pr,
        _ => return Err(not_found(id)),
    };

    // The last-admin guard only matters when revoking an `admin` assignment; for
    // any other role it is a no-op, so skip the tenant-wide FOR UPDATE lock and
    // do a plain soft-delete (avoids contention with concurrent admin changes).
    let revoked = if existing.role_id == ADMIN_ROLE_ID {
        person_roles_repo::try_soft_delete_protecting_last_admin(
            &state.db,
            id,
            ADMIN_ROLE_ID,
            reason.as_deref(),
        )
        .await
    } else {
        person_roles_repo::soft_delete(&state.db, tenant, id, reason.as_deref()).await
    }
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

/// `reason`, when present, must be at most 500 chars — mirrors the .NET
/// `MaximumLength(500)` on `CreatePersonRoleCommandValidator` /
/// `RevokeReasonValidator`.
fn reason_valid(reason: Option<&str>) -> bool {
    reason.is_none_or(|r| r.chars().count() <= MAX_REASON_LEN)
}

/// Build a 400 field-violation error (`invalid_argument`).
fn invalid_field(field: &str, message: &str, code: &str) -> CanonicalError {
    PersonRoleError::invalid_argument()
        .with_field_violation(field, message, code)
        .create()
}

fn reason_too_long() -> CanonicalError {
    invalid_field(
        "reason",
        "reason must be at most 500 characters",
        "invalid_reason",
    )
}

/// Clamp `?limit=` to `[1, 500]`; negatives → 1, absent → 50 (parity with the
/// .NET `int?` clamp — a nonsense value never 400s the request).
fn clamp_limit(limit: Option<i64>) -> u64 {
    limit.map_or(LIST_DEFAULT_LIMIT, |l| {
        u64::try_from(l).unwrap_or(1).clamp(1, LIST_MAX_LIMIT)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_length_validation() {
        assert!(reason_valid(None), "absent ok");
        assert!(reason_valid(Some("short")), "short ok");
        assert!(reason_valid(Some(&"x".repeat(MAX_REASON_LEN))), "500 ok");
        assert!(
            !reason_valid(Some(&"x".repeat(MAX_REASON_LEN + 1))),
            "501 too long"
        );
    }

    #[test]
    fn limit_clamping() {
        assert_eq!(clamp_limit(None), LIST_DEFAULT_LIMIT);
        assert_eq!(clamp_limit(Some(10)), 10);
        assert_eq!(clamp_limit(Some(0)), 1, "zero → 1");
        assert_eq!(clamp_limit(Some(-5)), 1, "negative → 1");
        assert_eq!(clamp_limit(Some(9999)), LIST_MAX_LIMIT, "over cap → 500");
    }
}
