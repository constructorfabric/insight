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
use axum::extract::{Extension, Path, Query};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use chrono::Utc;
use serde::Deserialize;
use toolkit_canonical_errors::CanonicalError;

use super::error::ConnectorHealthError;
use super::{ADMIN_ONLY, AppState, require_admin};
use crate::domain::connector_health::{
    ConnectorHealthResponse, ConnectorName, HISTORY_WINDOW, SourceId, SyncHistoryResponse,
    TenantId, read_health, read_syncs,
};

/// Which installation of the connector the window is asked for.
///
/// Both or neither: a source id is unique within a tenant, so half an identity
/// would narrow the window to rows from whichever tenant happened to share it.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SyncScopeQuery {
    /// Tenant of the instance. Omit both to span every instance of the
    /// connector.
    pub tenant_id: Option<String>,
    /// Source id of the instance, as its Secret annotates it.
    pub source_id: Option<String>,
}

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
    Query(scope): Query<SyncScopeQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let name = ConnectorName::parse(&connector).ok_or_else(unnamed_connector)?;
    let instance = parse_scope(&scope)?;
    let syncs = read_syncs(&state.ch, name.as_str(), instance.as_ref())
        .await
        .map_err(read_error)?;

    let (tenant_id, source_id) = match instance {
        Some((tenant, source)) => (Some(tenant.into_string()), Some(source.into_string())),
        None => (None, None),
    };
    Ok(Json(SyncHistoryResponse::build(
        name.into_string(),
        tenant_id,
        source_id,
        syncs,
        HISTORY_WINDOW,
    )))
}

/// Both halves or neither.
///
/// One alone is refused rather than ignored: silently widening the window back
/// to the whole connector would answer a question about one instance with rows
/// from all of them, and the answer would look right.
fn parse_scope(scope: &SyncScopeQuery) -> Result<Option<(TenantId, SourceId)>, CanonicalError> {
    match (scope.tenant_id.as_deref(), scope.source_id.as_deref()) {
        (None, None) => Ok(None),
        (Some(tenant), Some(source)) => {
            let tenant = TenantId::parse(tenant).ok_or_else(|| unnamed_instance("tenant_id"))?;
            let source = SourceId::parse(source).ok_or_else(|| unnamed_instance("source_id"))?;
            Ok(Some((tenant, source)))
        }
        (Some(_), None) => Err(half_an_instance("source_id")),
        (None, Some(_)) => Err(half_an_instance("tenant_id")),
    }
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

fn unnamed_instance(field: &'static str) -> CanonicalError {
    ConnectorHealthError::invalid_argument()
        .with_field_violation(
            field,
            "lowercase letters, digits and hyphens only",
            "INVALID",
        )
        .create()
}

fn half_an_instance(missing: &'static str) -> CanonicalError {
    ConnectorHealthError::invalid_argument()
        .with_field_violation(
            missing,
            "required alongside the other half of the instance identity",
            "REQUIRED",
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
