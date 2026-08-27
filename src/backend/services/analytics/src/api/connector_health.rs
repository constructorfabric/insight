//! Connector health handlers — `/v1/connector-health*`.
//!
//! Both routes are operator-gated and instance-wide: bronze schemas are not
//! tenant-partitioned, so the surface cannot be scoped by tenant and is gated
//! on the admin role instead. Neither handler calls anything but the warehouse,
//! and neither assembles its own answer — the response shapes are pure
//! functions over recorded facts, so what the page claims is decided somewhere
//! a test can reach without an `AppState`.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use chrono::Utc;
use toolkit_canonical_errors::CanonicalError;

use super::error::ConnectorHealthError;
use super::{ADMIN_ONLY, AppState, require_admin};
use crate::domain::connector_health::{
    ConnectorHealthResponse, ConnectorName, HISTORY_WINDOW, SyncHistoryResponse, read_health,
    read_syncs,
};

pub async fn get_connector_health(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let facts = read_health(&state.ch).await.map_err(read_error)?;

    Ok(Json(ConnectorHealthResponse::from_facts(facts, Utc::now())))
}

pub async fn get_connector_syncs(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(connector): Path<String>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let name = ConnectorName::parse(&connector).ok_or_else(unnamed_connector)?;
    let syncs = read_syncs(&state.ch, name.as_str())
        .await
        .map_err(read_error)?;

    Ok(Json(SyncHistoryResponse::build(
        name.into_string(),
        syncs,
        HISTORY_WINDOW,
    )))
}

/// Names the surface it refused, so an operator who followed a link knows what
/// to ask for rather than guessing which page rejected them.
fn admin_only() -> CanonicalError {
    ConnectorHealthError::permission_denied()
        .with_reason(ADMIN_ONLY)
        .create()
}

fn unnamed_connector() -> CanonicalError {
    ConnectorHealthError::invalid_argument()
        .with_field_violation(
            "connector",
            "lowercase letters, digits and hyphens only",
            "INVALID",
        )
        .create()
}

/// The reader's own failure never names the relation it could not read: a
/// warehouse error message on an admin surface is still a warehouse error
/// message on the wire.
#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn read_error(error: clickhouse::error::Error) -> CanonicalError {
    tracing::error!(error = %error, "connector health read failed");
    CanonicalError::internal("failed to read connector health").create()
}

#[cfg(test)]
mod tests;
