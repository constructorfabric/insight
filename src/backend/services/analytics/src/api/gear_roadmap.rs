//! Gear roadmap handler — `GET /v1/gear-roadmap`.
//!
//! Reads the delivery board out of bronze. Bronze schemas are not
//! tenant-partitioned, so the surface is gated on the admin role rather than
//! scoped by tenant, exactly as the connector-health reads are. The answer is a
//! pure function over the rows, so what the pages claim is decided somewhere a
//! test can reach without an `AppState`.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use chrono::Utc;
use toolkit_canonical_errors::CanonicalError;

use super::error::GearRoadmapError;
use super::{ADMIN_ONLY, AppState, require_admin};
use crate::domain::gear_roadmap::read::read_gears;
use crate::domain::gear_roadmap::response;
use crate::domain::gear_roadmap::sort::Sort;

pub async fn get_gear_roadmap(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Query(sort): Query<Sort>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let project_number = state.config.gear_roadmap_project_number;

    if project_number <= 0 {
        return Err(board_not_configured());
    }

    let gears = read_gears(&state.ch, project_number)
        .await
        .map_err(read_error)?;

    Ok(Json(response::build(
        &gears,
        Utc::now().date_naive(),
        sort,
        &state.external_links,
    )))
}

fn admin_only() -> CanonicalError {
    GearRoadmapError::permission_denied()
        .with_reason(ADMIN_ONLY)
        .create()
}

fn board_not_configured() -> CanonicalError {
    GearRoadmapError::failed_precondition()
        .with_precondition_violation(
            "gear roadmap board",
            "This deployment has not named the project the roadmap reads.",
            "BOARD_UNSET",
        )
        .create()
}

#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn read_error(error: clickhouse::error::Error) -> CanonicalError {
    tracing::error!(error = %error, "gear roadmap read failed");
    CanonicalError::internal("failed to read the gear roadmap").create()
}
