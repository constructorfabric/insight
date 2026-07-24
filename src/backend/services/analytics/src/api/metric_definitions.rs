//! `GET /v1/metric-definitions` HTTP handler.
//!
//! Read-only listing of the unified metric definitions for display surfaces
//! (metric catalog page). Tenant scope is resolved server-side from the
//! session `SecurityContext` — the endpoint takes no request parameters.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::response::IntoResponse;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::AppState;
use crate::domain::metric_definitions::listing;

/// `GET /v1/metric-definitions` handler.
///
/// # Errors
///
/// - `500 internal` — database failure or corrupt definition configuration.
pub async fn list_metric_definitions(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    let response = listing::list_definition_views(&state.db, ctx.subject_tenant_id()).await?;
    Ok(Json(response))
}
