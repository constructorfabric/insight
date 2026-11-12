mod connector_health;
mod connector_schedules;
mod error;
mod ingestion;

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode, header};
use axum::{Extension, Router};
use sea_orm::DatabaseConnection;
use toolkit::api::{OpenApiInfo, OpenApiRegistry, OpenApiRegistryImpl, OperationBuilder};
use toolkit_canonical_errors::CanonicalError;

use crate::domain::connector_health as connector_health_domain;
use crate::infra::identity::IdentityClient;

pub(crate) const ADMIN_ONLY: &str = "admin role required for this operation";

pub(crate) struct AppState {
    pub(crate) db: DatabaseConnection,
    pub(crate) ch: insight_clickhouse::Client,
    pub(crate) identity: IdentityClient,
    pub(crate) cluster: crate::infra::cluster::Cluster,
    pub(crate) schedule_slots: tokio::sync::Semaphore,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AppState").finish_non_exhaustive()
    }
}

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

fn build_operations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router = router;

    // Connector health: the operator's view of what the mover reports about
    // every connector's syncs. Admin-gated inside the handler, like every other
    // instance-wide read here.
    router = OperationBuilder::get("/v1/connector-health")
        .operation_id("analytics_api.connector_health.summary")
        .summary("Recorded sync state of every connector")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<connector_health_domain::ConnectorHealthResponse>(
            openapi,
            StatusCode::OK,
            "One row per connector, ordered by what needs acting on",
        )
        .standard_errors(openapi)
        .handler(connector_health::get_connector_health)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/connector-health/{connector}/syncs")
        .operation_id("analytics_api.connector_health.syncs")
        .summary("Recent syncs of one connector, newest first")
        .authenticated()
        .no_license_required()
        .path_param("connector", "Connector name, as the descriptors spell it")
        .query_param_typed(
            "tenant_id",
            false,
            "Tenant of one installation of the connector. Required alongside \
             source_id; omit both to span every installation",
            "string",
        )
        .query_param_typed(
            "source_id",
            false,
            "Source id of one installation, as its Secret annotates it. \
             Required alongside tenant_id",
            "string",
        )
        .json_response_with_schema::<connector_health_domain::SyncHistoryResponse>(
            openapi,
            StatusCode::OK,
            "A bounded window of recorded syncs",
        )
        .standard_errors(openapi)
        .handler(connector_health::get_connector_syncs)
        .register(router, openapi);

    // Ingestion intensity (ops). Admin-gated inside the handler, like the usage
    // read model: bronze rows carry no tenant, so this surface is
    // infrastructure-wide and cannot be scoped by the caller's tenant.
    router = OperationBuilder::get("/v1/ingestion/intensity")
        .operation_id("analytics_api.ingestion.intensity")
        .summary("Bronze extraction intensity per bucket")
        .authenticated()
        .no_license_required()
        .query_param_typed("grain", false, "Bucket width: 15m or 1s", "string")
        .query_param_typed(
            "scope",
            false,
            "Bronze database to scope to, e.g. bronze_bamboohr",
            "string",
        )
        .query_param_typed(
            "series",
            false,
            "What one band counts: connector, stream or total",
            "string",
        )
        .query_param_typed("from", false, "Inclusive lower bound, RFC 3339", "string")
        .query_param_typed("to", false, "Exclusive upper bound, RFC 3339", "string")
        .json_response_with_schema::<ingestion::IngestionIntensityResponse>(
            openapi,
            StatusCode::OK,
            "Extraction intensity buckets",
        )
        .standard_errors(openapi)
        .handler(ingestion::get_ingestion_intensity)
        .register(router, openapi);

    connector_schedules::register(router, openapi)
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

#[cfg(test)]
mod openapi_tests;
