mod error;

use std::sync::Arc;

use axum::http::{HeaderMap, header};
use axum::{Extension, Router};
use sea_orm::DatabaseConnection;
use toolkit::api::{OpenApiInfo, OpenApiRegistry, OpenApiRegistryImpl};
use toolkit_canonical_errors::CanonicalError;

use crate::infra::identity::IdentityClient;

#[expect(dead_code, reason = "the route table is empty")]
pub(crate) struct AppState {
    pub(crate) db: DatabaseConnection,
    pub(crate) ch: insight_clickhouse::Client,
    pub(crate) identity: IdentityClient,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AppState").finish_non_exhaustive()
    }
}

#[cfg_attr(not(test), expect(dead_code, reason = "the route table is empty"))]
pub(crate) async fn require_admin(
    state: &AppState,
    headers: &HeaderMap,
    denied: fn() -> CanonicalError,
) -> Result<(), CanonicalError> {
    if !state.identity.is_configured() {
        tracing::error!("identity service is not configured; admin access cannot be verified");
        return Err(error::identity_unavailable());
    }
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let admin = state
        .identity
        .is_admin(authorization)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "admin role check failed");
            error::identity_unavailable()
        })?;
    if admin {
        return Ok(());
    }
    Err(denied())
}

pub(crate) fn register_routes(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let api = build_operations(Router::new(), openapi)
        .layer(Extension(state))
        .layer(insight_http_metrics::ServerMetricsLayer::new("connectors"))
        .layer(insight_log_context::LogContextLayer::new());
    host_router.merge(api)
}

fn build_operations(router: Router, _openapi: &dyn OpenApiRegistry) -> Router {
    router
}

pub(crate) fn openapi_document() -> anyhow::Result<utoipa::openapi::OpenApi> {
    let registry = OpenApiRegistryImpl::new();
    let _ = build_operations(Router::new(), &registry);
    registry
        .build_openapi(&OpenApiInfo {
            title: "Connectors API".to_owned(),
            version: "1.0.0".to_owned(),
            description: Some(
                "Connector management service. Mounted at /api/connectors.".to_owned(),
            ),
            servers: Vec::new(),
        })
        .map_err(|error| anyhow::anyhow!("failed to build connectors OpenAPI document: {error}"))
}

#[cfg(test)]
mod admin_tests;
#[cfg(test)]
mod log_context_tests;
#[cfg(test)]
mod log_leak_tests;
