//! HTTP API layer — routes and handlers.

pub(crate) mod canonical_json;
mod catalog;
pub(crate) mod error;
mod handlers;

#[cfg(test)]
mod tenant_resolution_tests;

use axum::{Router, middleware};
use sea_orm::DatabaseConnection;
use std::sync::Arc;

use crate::auth;
use crate::config::AppConfig;
use crate::domain::auth::TenantAuthorization;
use crate::domain::catalog::CatalogReader;
use crate::domain::schema_validator::SchemaValidator;
use crate::infra::identity::IdentityClient;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub ch: insight_clickhouse::Client,
    pub identity: IdentityClient,
    #[allow(dead_code)] // will be used for runtime config access (rate limits, feature flags)
    pub config: AppConfig,
    /// Schema-validator (Refs #521). Held in `AppState` so admin-crud (#525)
    /// can call `validator.validate(metric_key)` after a successful threshold
    /// write. Not currently consumed by any handler in this PR.
    #[allow(dead_code)] // wired in #525; #521 only exposes the function
    pub validator: SchemaValidator,
    /// Catalog auth-trait (Refs #522). Resolves session-bound tenant against
    /// the operator-configured single-tenant fallback per
    /// `cpt-metric-cat-constraint-tenant-default`. Consumed by
    /// `auth::tenant_middleware`; #525 will consume it directly for
    /// `is_tenant_admin` / `actor_subject` (out of scope here).
    pub tenant_auth: Arc<dyn TenantAuthorization>,
    /// Catalog read pipeline (Refs #524) — cache + resolver wired together.
    /// Cheap to clone (internally `Arc`s the cache + resolver).
    pub catalog_reader: CatalogReader,
}

/// Build the Axum router with all routes.
pub fn router(state: AppState) -> Router {
    let state = Arc::new(state);

    let router = Router::new()
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
        .route(
            "/v1/metrics/queries",
            axum::routing::post(handlers::query_metrics_batch),
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
        // Person lookup (delegates to Identity service)
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
        // Metric catalog read (Refs #524) — DESIGN §3.3 "Catalog Read".
        // POST chosen so request-context fields (role_slug, team_id) never
        // appear in HTTP access logs / proxy captures, and so HTTP / CDN
        // intermediaries cannot cache the response (server-side cache is the
        // single canonical cache layer per `cpt-metric-cat-principle-server-cache`).
        .route(
            "/catalog/get_metrics",
            axum::routing::post(catalog::get_metrics),
        )
        // Health
        .route("/health", axum::routing::get(handlers::health));

    // The tenant-resolution middleware uses just the auth-trait — not full
    // `AppState` — as its layer state, so the integration tests in
    // `tenant_resolution_tests` can mount it without standing up a
    // `DatabaseConnection`. The `Arc<dyn TenantAuthorization>` is cloned
    // here and again handed to the route state via `AppState`.
    let tenant_auth = state.tenant_auth.clone();

    router
        .layer(middleware::from_fn_with_state(
            tenant_auth,
            auth::tenant_middleware,
        ))
        .with_state(state)
}
