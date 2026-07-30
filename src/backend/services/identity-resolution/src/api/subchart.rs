//! Org subchart HTTP surface — single-root subtree + forest (#348 / #344).
//!
//! Ported 1:1 from the .NET `SubchartEndpoints`. Both routes are authenticated
//! (any identified caller); what they return is shaped by the caller's
//! visibility grants. `GET /v1/subchart/{person_id}` gates the root through
//! [`crate::infra::db::subchart_repo::is_target_in_visible_set`] and returns 404
//! (not 403) on deny so the target's existence does not leak. `GET /v1/subchart`
//! is the forest variant — every visible top, empty array (200) when the caller
//! sees nothing. `depth` (>= 0) and `valid_at` (point-in-time lens, #582) mirror
//! the .NET query contract, except `depth` is **capped at the server's
//! `max_depth`** (and defaults to it when omitted) — the `UNION ALL` subtree CTEs
//! would otherwise let a caller pull a whole large-tenant tree in one request, or
//! recurse until `cte_max_recursion_depth` on cyclic `org_chart` data. (The
//! visibility gate's CTE is `UNION`/distinct, so it self-terminates on cycles.)

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, Query};
use axum::response::IntoResponse;
use chrono::{NaiveDateTime, TimeDelta, Utc};
use serde::Deserialize;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::AppState;
use super::datetime::parse_flexible;
use super::error::SubchartError;
use super::gate::require_caller;
use crate::domain::subchart::assemble_forest;
// Re-export the response DTOs so the route table (`api::mod`) can reference them
// as `subchart::SubchartResponse`, alongside the handlers they wrap.
pub(crate) use crate::domain::subchart::{SubchartForestResponse, SubchartResponse};
use crate::infra::db::subchart_repo;

/// Query params shared by both subchart routes.
#[derive(Debug, Deserialize)]
pub struct SubchartParams {
    /// Max descent depth; `>= 0`. Capped at the server's `max_depth` and
    /// defaulted to it when omitted (see [`effective_depth`]).
    pub depth: Option<i64>,
    /// Point-in-time lens (ISO-8601 / RFC-3339). Absent = current state.
    pub valid_at: Option<String>,
}

/// `GET /v1/subchart` — forest of every root the caller can see.
pub async fn get_forest(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(params): Query<SubchartParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    let caller = require_caller(&ctx)?;
    let tenant = ctx.subject_tenant_id();
    let max_depth = Some(effective_depth(
        validate_depth(params.depth)?,
        state.config.max_depth,
    ));
    let valid_at = resolve_valid_at(params.valid_at.as_deref())?;
    let source = &state.config.org_chart_source_type;

    let flat =
        subchart_repo::get_forest_flat(&state.db, tenant, caller, source, max_depth, valid_at)
            .await
            .map_err(read_err)?;
    Ok(Json(SubchartForestResponse {
        roots: assemble_forest(flat),
    }))
}

/// `GET /v1/subchart/{person_id}` — subtree rooted at a person the caller can
/// see. 404 when the root is unknown OR not visible to the caller.
pub async fn get_subchart(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(person_id): Path<Uuid>,
    Query(params): Query<SubchartParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    let caller = require_caller(&ctx)?;
    let tenant = ctx.subject_tenant_id();
    let max_depth = Some(effective_depth(
        validate_depth(params.depth)?,
        state.config.max_depth,
    ));
    let valid_at = resolve_valid_at(params.valid_at.as_deref())?;
    let source = &state.config.org_chart_source_type;

    let can_see = subchart_repo::is_target_in_visible_set(
        &state.db, tenant, caller, person_id, source, valid_at,
    )
    .await
    .map_err(read_err)?;
    if !can_see {
        return Err(not_found(person_id));
    }

    let flat =
        subchart_repo::get_subchart_flat(&state.db, tenant, person_id, source, max_depth, valid_at)
            .await
            .map_err(read_err)?;
    match assemble_forest(flat).into_iter().next() {
        Some(root) => Ok(Json(SubchartResponse { root })),
        None => Err(not_found(person_id)),
    }
}

fn not_found(person_id: Uuid) -> CanonicalError {
    SubchartError::not_found(format!("person {person_id} not found or not visible"))
        .with_resource(person_id.to_string())
        .create()
}

// Takes the error by value so it can be used directly as `.map_err(read_err)`.
#[allow(clippy::needless_pass_by_value)]
fn read_err(e: anyhow::Error) -> CanonicalError {
    tracing::error!(error = %e, "subchart query failed");
    CanonicalError::internal("failed to read subchart").create()
}

/// Validate the `depth` query param: `None` → unspecified (the caller omitted
/// it — [`effective_depth`] then applies the server cap); negative → 400
/// `invalid_depth`; out-of-`i32`-range → 400. Mirrors the .NET `depth is < 0`
/// guard (with the extra range check the .NET `int?` binder does implicitly).
fn validate_depth(depth: Option<i64>) -> Result<Option<i32>, CanonicalError> {
    match depth {
        None => Ok(None),
        Some(d) if d < 0 => Err(invalid_depth(format!("depth must be >= 0; got {d}"))),
        Some(d) => i32::try_from(d)
            .map(Some)
            .map_err(|_| invalid_depth(format!("depth is out of range: {d}"))),
    }
}

fn invalid_depth(detail: String) -> CanonicalError {
    SubchartError::invalid_argument()
        .with_field_violation("depth", detail, "invalid_depth")
        .create()
}

/// Resolve the effective descent cap: the caller's validated `depth` clamped to
/// the server's `cap` (`config.max_depth`), defaulting to `cap` when omitted.
/// Never `None`, so the `UNION ALL` subtree CTEs are always bounded — the caller
/// cannot force an unbounded traversal, and cyclic `org_chart` data returns a
/// bounded (partial) tree instead of erroring at `cte_max_recursion_depth`.
fn effective_depth(requested: Option<i32>, cap: usize) -> i32 {
    let cap = i32::try_from(cap).unwrap_or(i32::MAX);
    requested.map_or(cap, |d| d.min(cap))
}

/// Parse + validate the optional `valid_at`: normalise to naive-UTC and reject
/// future values (one-minute clock-skew slack), matching the .NET
/// `NormalizeValidAtToUtc` + `ValidateValidAtNotFuture`.
fn resolve_valid_at(raw: Option<&str>) -> Result<Option<NaiveDateTime>, CanonicalError> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let ts = parse_flexible(raw).ok_or_else(|| {
        invalid_valid_at(format!(
            "valid_at is not a recognised date/datetime: '{raw}'"
        ))
    })?;
    if ts > Utc::now().naive_utc() + TimeDelta::minutes(1) {
        return Err(invalid_valid_at(format!(
            "valid_at must not be in the future; got {ts}"
        )));
    }
    Ok(Some(ts))
}

fn invalid_valid_at(detail: String) -> CanonicalError {
    SubchartError::invalid_argument()
        .with_field_violation("valid_at", detail, "invalid_valid_at")
        .create()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_validation() {
        assert!(matches!(validate_depth(None), Ok(None)));
        assert!(matches!(validate_depth(Some(0)), Ok(Some(0))));
        assert!(matches!(validate_depth(Some(5)), Ok(Some(5))));
        assert!(validate_depth(Some(-1)).is_err(), "negative rejected");
        assert!(
            validate_depth(Some(i64::from(i32::MAX) + 1)).is_err(),
            "overflow rejected"
        );
    }

    #[test]
    fn depth_is_capped_by_server_max() {
        assert_eq!(effective_depth(None, 16), 16, "omitted → server cap");
        assert_eq!(effective_depth(Some(5), 16), 5, "under cap kept");
        assert_eq!(effective_depth(Some(100), 16), 16, "over cap clamped");
    }

    #[test]
    fn rejects_future_valid_at() {
        let future = (Utc::now().naive_utc() + TimeDelta::days(2))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        assert!(resolve_valid_at(Some(&future)).is_err(), "future rejected");
    }

    #[test]
    fn absent_or_blank_valid_at_is_none() {
        assert!(matches!(resolve_valid_at(None), Ok(None)));
        assert!(matches!(resolve_valid_at(Some("   ")), Ok(None)));
    }
}
