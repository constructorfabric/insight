//! Visibility grants HTTP surface — create / list / revoke.
//!
//! Admin-gated; ported 1:1 from the .NET `VisibilityEndpoints` (ADR-0012).
//! `viewed_person_id` null = viewer sees the whole tenant tree. Revoke is a
//! plain soft-delete (no lockout guard).

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
use super::error::VisibilityError;
use super::gate::require_admin;
use crate::infra::db::visibility_repo::{self, Visibility};

const LIST_DEFAULT_LIMIT: u64 = 50;
const LIST_MAX_LIMIT: u64 = 500;
const MAX_REASON_LEN: usize = 500; // VARCHAR(500)

/// Body of `POST /v1/visibility` — grant a viewer visibility over a target
/// (or the whole tree when `viewed_person_id` is omitted).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateVisibilityRequest {
    pub viewer_person_id: Uuid,
    #[serde(default)]
    pub viewed_person_id: Option<Uuid>,
    /// Optional grant start; defaults to now when omitted. Accepts RFC-3339
    /// (`Z`/offset), zone-less, or date-only, normalised to naive-UTC.
    #[serde(default, deserialize_with = "super::datetime::deserialize_opt")]
    pub valid_from: Option<DateTime>,
    #[serde(default)]
    pub reason: Option<String>,
}
impl toolkit::api::api_dto::RequestApiDto for CreateVisibilityRequest {}

/// One visibility grant.
#[derive(Debug, Serialize, ToSchema)]
pub struct VisibilityResponse {
    pub visibility_id: Uuid,
    pub insight_tenant_id: Uuid,
    pub viewer_person_id: Uuid,
    pub viewed_person_id: Option<Uuid>,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub author_person_id: Uuid,
    pub reason: Option<String>,
    pub created_at: String,
}
impl toolkit::api::api_dto::ResponseApiDto for VisibilityResponse {}

impl From<Visibility> for VisibilityResponse {
    fn from(v: Visibility) -> Self {
        Self {
            visibility_id: v.visibility_id,
            insight_tenant_id: v.insight_tenant_id,
            viewer_person_id: v.viewer_person_id,
            viewed_person_id: v.viewed_person_id,
            valid_from: fmt_ts(v.valid_from),
            valid_to: v.valid_to.map(fmt_ts),
            author_person_id: v.author_person_id,
            reason: v.reason,
            created_at: fmt_ts(v.created_at),
        }
    }
}

/// List wrapper.
#[derive(Debug, Serialize, ToSchema)]
pub struct VisibilityListResponse {
    pub items: Vec<VisibilityResponse>,
}
impl toolkit::api::api_dto::ResponseApiDto for VisibilityListResponse {}

/// Optional `DELETE` body carrying a revoke reason.
#[derive(Debug, Deserialize)]
pub struct RevokeReasonRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub viewer: Option<Uuid>,
    pub viewed: Option<Uuid>,
    pub active: Option<bool>,
    // Signed so a negative `?limit=` clamps to 1 (parity with the .NET `int?`
    // clamp) rather than failing query deserialization.
    pub limit: Option<i64>,
}

/// `POST /v1/visibility` — create a grant (admin only).
pub async fn create_visibility(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<CreateVisibilityRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let author = require_admin(&state.db, &ctx).await?;

    if req.viewer_person_id.is_nil() {
        return Err(VisibilityError::invalid_argument()
            .with_field_violation(
                "viewer_person_id",
                "viewer_person_id is required",
                "INVALID",
            )
            .create());
    }
    // A present-but-nil target is nonsense (a grant to see person 000…0); only an
    // ABSENT viewed_person_id means whole-tree visibility.
    if req.viewed_person_id == Some(Uuid::nil()) {
        return Err(VisibilityError::invalid_argument()
            .with_field_violation(
                "viewed_person_id",
                "viewed_person_id must not be the nil UUID; omit it for whole-tree visibility",
                "invalid_viewed_person_id",
            )
            .create());
    }
    if !reason_valid(req.reason.as_deref()) {
        return Err(reason_too_long());
    }

    let visibility_id = Uuid::now_v7();
    visibility_repo::insert(
        &state.db,
        visibility_id,
        tenant,
        req.viewer_person_id,
        req.viewed_person_id,
        req.valid_from,
        author,
        req.reason.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "insert visibility failed");
        CanonicalError::internal("failed to create grant").create()
    })?;
    tracing::info!(%visibility_id, viewer = %req.viewer_person_id, author_person_id = %author, "visibility.create");

    let created = visibility_repo::get_by_id(&state.db, tenant, visibility_id)
        .await
        .map_err(read_err)?
        .ok_or_else(|| CanonicalError::internal("grant vanished after insert").create())?;

    let location = format!("/v1/visibility/{visibility_id}");
    Ok((
        StatusCode::CREATED,
        [(LOCATION, location)],
        Json(VisibilityResponse::from(created)),
    ))
}

/// `GET /v1/visibility` — list grants (admin only). Filters: `?viewer=`,
/// `?viewed=`, `?active=` (default all), `?limit=` (default 50, cap 500).
pub async fn list_visibility(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    require_admin(&state.db, &ctx).await?;

    let limit = clamp_limit(params.limit);
    let rows = visibility_repo::list(
        &state.db,
        tenant,
        params.viewer,
        params.viewed,
        params.active.unwrap_or(false),
        limit,
    )
    .await
    .map_err(read_err)?;
    let items = rows.into_iter().map(VisibilityResponse::from).collect();
    Ok(Json(VisibilityListResponse { items }))
}

/// `DELETE /v1/visibility/{id}` — revoke a grant (admin only).
pub async fn delete_visibility(
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

    // 404 only if the grant never existed; otherwise soft-delete + 204 (a
    // second revoke of an already-revoked grant is a no-op 204). Parity w/ .NET.
    if visibility_repo::get_by_id(&state.db, tenant, id)
        .await
        .map_err(read_err)?
        .is_none()
    {
        return Err(VisibilityError::not_found("visibility not found")
            .with_resource(id.to_string())
            .create());
    }

    let rows = visibility_repo::soft_delete(&state.db, tenant, id, reason.as_deref())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "revoke visibility failed");
            CanonicalError::internal("failed to revoke grant").create()
        })?;
    tracing::info!(visibility_id = %id, rows_affected = rows, author_person_id = %author, "visibility.revoke");
    Ok(StatusCode::NO_CONTENT)
}

// Takes the error by value so it can be used directly as `.map_err(read_err)`.
#[allow(clippy::needless_pass_by_value)]
fn read_err(e: anyhow::Error) -> CanonicalError {
    tracing::error!(error = %e, "visibility query failed");
    CanonicalError::internal("failed to read grants").create()
}

/// `reason`, when present, must be at most 500 chars — mirrors the .NET
/// `MaximumLength(500)` on the create + `RevokeReasonValidator`.
fn reason_valid(reason: Option<&str>) -> bool {
    reason.is_none_or(|r| r.chars().count() <= MAX_REASON_LEN)
}

fn reason_too_long() -> CanonicalError {
    VisibilityError::invalid_argument()
        .with_field_violation(
            "reason",
            "reason must be at most 500 characters",
            "invalid_reason",
        )
        .create()
}

/// Clamp `?limit=` to `[1, 500]`; negatives → 1, absent → 50 (parity with the
/// .NET `int?` clamp).
fn clamp_limit(limit: Option<i64>) -> u64 {
    limit.map_or(LIST_DEFAULT_LIMIT, |l| {
        u64::try_from(l).unwrap_or(1).clamp(1, LIST_MAX_LIMIT)
    })
}

/// Format a DB `DateTime` (naive) as ISO-8601 with a `T` separator.
fn fmt_ts(dt: DateTime) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
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
