//! Gear roadmap handler — `GET /v1/gear-roadmap`.
//!
//! Reads the delivery board out of bronze. Bronze schemas are not
//! tenant-partitioned, so the surface is gated on the admin role rather than
//! scoped by tenant, exactly as the connector-health reads are. The answer is a
//! pure function over the rows, so what the pages claim is decided somewhere a
//! test can reach without an `AppState`.
//!
//! The caller names the board; `GET /v1/gear-roadmap/boards` lists them.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use chrono::Utc;
use serde::Deserialize;
use toolkit_canonical_errors::CanonicalError;

use super::error::GearRoadmapError;
use super::{ADMIN_ONLY, AppState, require_admin};
use crate::domain::gear_roadmap::boards;
use crate::domain::gear_roadmap::read::read_gears;
use crate::domain::gear_roadmap::response;
use crate::domain::gear_roadmap::sort::{Direction, GearSort, Sort};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardRequest {
    project: i64,
    #[serde(default)]
    sort: GearSort,
    #[serde(default)]
    direction: Direction,
}

pub async fn get_gear_roadmap(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Query(request): Query<BoardRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    if request.project <= 0 {
        return Err(board_not_a_board());
    }

    let gears = read_gears(&state.ch, request.project)
        .await
        .map_err(read_error)?;

    Ok(Json(response::build(
        &gears,
        Utc::now().date_naive(),
        Sort {
            sort: request.sort,
            direction: request.direction,
        },
        &state.external_links,
    )))
}

pub async fn get_gear_roadmap_boards(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let rows = boards::read_boards(&state.ch).await.map_err(read_error)?;

    Ok(Json(boards::build(rows)))
}

fn admin_only() -> CanonicalError {
    GearRoadmapError::permission_denied()
        .with_reason(ADMIN_ONLY)
        .create()
}

fn board_not_a_board() -> CanonicalError {
    GearRoadmapError::invalid_argument()
        .with_field_violation(
            "project",
            "A board is named by its positive project number.",
            "INVALID",
        )
        .create()
}

#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn read_error(error: clickhouse::error::Error) -> CanonicalError {
    tracing::error!(error = %error, "gear roadmap read failed");
    CanonicalError::internal("failed to read the gear roadmap").create()
}
