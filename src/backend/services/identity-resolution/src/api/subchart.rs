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
pub(crate) use crate::domain::subchart::{SubchartForestResponse, SubchartResponse};
use crate::infra::db::subchart_repo;

#[derive(Debug, Deserialize)]
pub struct SubchartParams {
    pub depth: Option<i64>,
    pub valid_at: Option<String>,
}

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

    let flat = subchart_repo::get_forest_flat(
        &state.db,
        tenant,
        caller,
        source,
        max_depth,
        valid_at,
        state.config.visibility_policy,
    )
    .await
    .map_err(read_err)?;
    Ok(Json(SubchartForestResponse {
        roots: assemble_forest(flat),
    }))
}

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
        &state.db,
        tenant,
        caller,
        person_id,
        source,
        valid_at,
        state.config.visibility_policy,
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
#[expect(clippy::needless_pass_by_value)]
fn read_err(e: anyhow::Error) -> CanonicalError {
    tracing::error!(error = %e, "subchart query failed");
    CanonicalError::internal("failed to read subchart").create()
}

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

fn effective_depth(requested: Option<i32>, cap: usize) -> i32 {
    let cap = i32::try_from(cap).unwrap_or(i32::MAX);
    requested.map_or(cap, |d| d.min(cap))
}

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
