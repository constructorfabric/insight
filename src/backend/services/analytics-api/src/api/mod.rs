//! HTTP API layer — routes and handlers.

mod handlers;

use axum::{Router, middleware};
use sea_orm::DatabaseConnection;
use std::sync::Arc;

use crate::auth;
use crate::config::AppConfig;
use crate::infra::identity_resolution::IdentityResolutionClient;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub ch: insight_clickhouse::Client,
    pub identity: IdentityResolutionClient,
    #[allow(dead_code)] // will be used for runtime config access (rate limits, feature flags)
    pub config: AppConfig,
}

/// Build the Axum router with all routes.
pub fn router(state: AppState) -> Router {
    let state = Arc::new(state);

    Router::new()
        // Metric CRUD
        .route("/v1/metrics", axum::routing::get(handlers::list_metrics))
        .route("/v1/metrics", axum::routing::post(handlers::create_metric))
        .route("/v1/metrics/{id}", axum::routing::get(handlers::get_metric))
        .route(
            "/v1/metrics/{id}",
            axum::routing::put(handlers::update_metric),
        )
        .route(
            "/v1/metrics/{id}",
            axum::routing::delete(handlers::delete_metric),
        )
        // Query
        .route(
            "/v1/metrics/{id}/query",
            axum::routing::post(handlers::query_metric),
        )
        // Thresholds
        .route(
            "/v1/metrics/{id}/thresholds",
            axum::routing::get(handlers::list_thresholds),
        )
        .route(
            "/v1/metrics/{id}/thresholds",
            axum::routing::post(handlers::create_threshold),
        )
        .route(
            "/v1/metrics/{id}/thresholds/{tid}",
            axum::routing::put(handlers::update_threshold),
        )
        .route(
            "/v1/metrics/{id}/thresholds/{tid}",
            axum::routing::delete(handlers::delete_threshold),
        )
        // Person lookup (delegates to Identity Resolution service)
        .route(
            "/v1/persons/{email}",
            axum::routing::get(handlers::get_person),
        )
        // Column catalog
        .route("/v1/columns", axum::routing::get(handlers::list_columns))
        .route(
            "/v1/columns/{table}",
            axum::routing::get(handlers::list_columns_for_table),
        )
        // Health
        .route("/health", axum::routing::get(handlers::health))
        // Auth middleware on all routes
        .layer(middleware::from_fn(auth::auth_middleware))
        .with_state(state)
}
