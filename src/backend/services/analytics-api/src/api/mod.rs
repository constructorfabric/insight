//! HTTP API layer — routes and handlers.

pub(crate) mod admin;
pub(crate) mod canonical_json;
mod catalog;
pub(crate) mod error;
mod handlers;

#[cfg(test)]
mod tenant_resolution_tests;

use axum::http::StatusCode;
use axum::middleware::from_fn;
use axum::{Extension, Router};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use toolkit::api::{OpenApiRegistry, OperationBuilder};

use crate::auth;
use crate::config::GearConfig;
use crate::domain::admin_threshold::AdminThresholdService;
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
    pub config: GearConfig,
    /// Schema-validator (Refs #521). Held in `AppState` so admin-crud (#525)
    /// calls `validator.validate(metric_key)` after a successful threshold
    /// write. Kept on `AppState` for the legacy /v1/metrics handlers'
    /// future use too; admin-crud receives its own clone via
    /// [`AdminThresholdService::new`].
    #[allow(dead_code)] // admin-crud holds its own clone; #521 only exposes the function
    pub validator: SchemaValidator,
    /// Catalog auth-trait. Resolves session-bound tenant against the
    /// operator-configured single-tenant fallback per
    /// `cpt-metric-cat-constraint-tenant-default` (Refs #522). The
    /// `AdminThresholdService` holds its own clone for `is_tenant_admin` /
    /// `actor_subject`; this field retains the handle for future per-request
    /// authz wiring once auth is re-enabled on this host.
    #[allow(dead_code)] // admin-crud holds its own clone; retained for re-enabling auth
    pub tenant_auth: Arc<dyn TenantAuthorization>,
    /// Catalog read pipeline (Refs #524) — cache + resolver wired together.
    /// Cheap to clone (internally `Arc`s the cache + resolver).
    pub catalog_reader: CatalogReader,
    /// Admin-CRUD service (Refs #525) — owns the 5 `/v1/admin/metric-thresholds/*`
    /// endpoints, the validation gauntlet, the `lock-enforcer` SQL, and the
    /// `audit-emitter` dual-sink contract.
    pub admin_threshold: AdminThresholdService,
}

/// Register all analytics-api routes onto the host's stateless router.
///
/// Routes are declared through the toolkit's [`OperationBuilder`], so each
/// endpoint records an `OpenAPI` `OperationSpec` plus auth/license metadata in
/// the host-provided `openapi` registry (the gears-rust idiom — see
/// `gears/file-parser`).
///
/// The router is the host's stateless `Router` (route state `()`), so handlers
/// take `Extension<Arc<AppState>>` rather than `State<_>`. The shared
/// `Arc<AppState>` is attached via `router.layer(Extension(state))`, and the
/// per-request tenant override runs via `router.layer(from_fn(tenant_middleware))`.
///
/// `OperationBuilder::register` merges method routers per path, so the
/// shared-path endpoints (`/v1/metrics`, `/v1/admin/metric-thresholds*`) are
/// registered as independent operations.
// One `OperationBuilder` chain per endpoint makes this a long-but-flat route
// table; splitting it across helpers would only obscure the 1:1 route↔handler map.
#[allow(clippy::too_many_lines)]
pub fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let mut router: Router = router;

    // Metric CRUD
    router = OperationBuilder::get("/v1/metrics")
        .operation_id("analytics_api.metrics.list")
        .summary("List metrics")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "List of metrics")
        .standard_errors(openapi)
        .handler(handlers::list_metrics)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/metrics")
        .operation_id("analytics_api.metrics.create")
        .summary("Create a metric")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::CREATED, "Created metric")
        .standard_errors(openapi)
        .handler(handlers::create_metric)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/metrics/{id}")
        .operation_id("analytics_api.metrics.get")
        .summary("Get a metric by id")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "Metric")
        .standard_errors(openapi)
        .handler(handlers::get_metric)
        .register(router, openapi);

    router = OperationBuilder::put("/v1/metrics/{id}")
        .operation_id("analytics_api.metrics.update")
        .summary("Update a metric")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "Updated metric")
        .standard_errors(openapi)
        .handler(handlers::update_metric)
        .register(router, openapi);

    router = OperationBuilder::delete("/v1/metrics/{id}")
        .operation_id("analytics_api.metrics.delete")
        .summary("Delete a metric")
        .authenticated()
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Metric deleted")
        .standard_errors(openapi)
        .handler(handlers::delete_metric)
        .register(router, openapi);

    // Query
    router = OperationBuilder::post("/v1/metrics/{id}/query")
        .operation_id("analytics_api.metrics.query")
        .summary("Query a single metric")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "Query result")
        .standard_errors(openapi)
        .handler(handlers::query_metric)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/metrics/queries")
        .operation_id("analytics_api.metrics.query_batch")
        .summary("Query metrics in batch")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "Batch query result")
        .standard_errors(openapi)
        .handler(handlers::query_metrics_batch)
        .register(router, openapi);

    // Thresholds (legacy)
    router = OperationBuilder::get("/v1/metrics/{id}/thresholds")
        .operation_id("analytics_api.thresholds.list")
        .summary("List thresholds for a metric")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "List of thresholds")
        .standard_errors(openapi)
        .handler(handlers::list_thresholds)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/metrics/{id}/thresholds")
        .operation_id("analytics_api.thresholds.create")
        .summary("Create a threshold for a metric")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::CREATED, "Created threshold")
        .standard_errors(openapi)
        .handler(handlers::create_threshold)
        .register(router, openapi);

    router = OperationBuilder::put("/v1/metrics/{id}/thresholds/{tid}")
        .operation_id("analytics_api.thresholds.update")
        .summary("Update a threshold")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "Updated threshold")
        .standard_errors(openapi)
        .handler(handlers::update_threshold)
        .register(router, openapi);

    router = OperationBuilder::delete("/v1/metrics/{id}/thresholds/{tid}")
        .operation_id("analytics_api.thresholds.delete")
        .summary("Delete a threshold")
        .authenticated()
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Threshold deleted")
        .standard_errors(openapi)
        .handler(handlers::delete_threshold)
        .register(router, openapi);

    // Person lookup (delegates to Identity service)
    router = OperationBuilder::get("/v1/persons/{email}")
        .operation_id("analytics_api.persons.get")
        .summary("Resolve a person by email")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "Person")
        .standard_errors(openapi)
        .handler(handlers::get_person)
        .register(router, openapi);

    // Column catalog
    router = OperationBuilder::get("/v1/columns")
        .operation_id("analytics_api.columns.list")
        .summary("List queryable columns")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "List of columns")
        .standard_errors(openapi)
        .handler(handlers::list_columns)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/columns/{table}")
        .operation_id("analytics_api.columns.list_for_table")
        .summary("List queryable columns for a table")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "List of columns")
        .standard_errors(openapi)
        .handler(handlers::list_columns_for_table)
        .register(router, openapi);

    // Metric catalog read (Refs #524) — DESIGN §3.3 "Catalog Read".
    // POST chosen so request-context fields (role_slug, team_id) never
    // appear in HTTP access logs / proxy captures, and so HTTP / CDN
    // intermediaries cannot cache the response (server-side cache is the
    // single canonical cache layer per `cpt-metric-cat-principle-server-cache`).
    router = OperationBuilder::post("/v1/catalog/get_metrics")
        .operation_id("analytics_api.catalog.get_metrics")
        .summary("Read the metric catalog for the request context")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "Resolved metric catalog")
        .standard_errors(openapi)
        .handler(catalog::get_metrics)
        .register(router, openapi);

    // Admin threshold CRUD (Refs #525) — DESIGN §3.2 admin-crud.
    // Bearer-token-only auth at the gateway (Q1 ack); the catalog
    // surface enforces canonical envelopes + CSRF closure via the
    // `CanonicalJson` extractor (Content-Type: application/json
    // required, deny_unknown_fields on every body shape).
    router = OperationBuilder::get("/v1/admin/metric-thresholds")
        .operation_id("analytics_api.admin.thresholds.list")
        .summary("List admin metric thresholds")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "List of metric thresholds")
        .standard_errors(openapi)
        .handler(admin::list)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/admin/metric-thresholds")
        .operation_id("analytics_api.admin.thresholds.create")
        .summary("Create an admin metric threshold")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::CREATED, "Created metric threshold")
        .standard_errors(openapi)
        .handler(admin::create)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/admin/metric-thresholds/{id}")
        .operation_id("analytics_api.admin.thresholds.get")
        .summary("Get an admin metric threshold by id")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "Metric threshold")
        .standard_errors(openapi)
        .handler(admin::get_one)
        .register(router, openapi);

    router = OperationBuilder::put("/v1/admin/metric-thresholds/{id}")
        .operation_id("analytics_api.admin.thresholds.update")
        .summary("Update an admin metric threshold")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "Updated metric threshold")
        .standard_errors(openapi)
        .handler(admin::update)
        .register(router, openapi);

    router = OperationBuilder::delete("/v1/admin/metric-thresholds/{id}")
        .operation_id("analytics_api.admin.thresholds.delete")
        .summary("Delete an admin metric threshold")
        .authenticated()
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Metric threshold deleted")
        .standard_errors(openapi)
        .handler(admin::delete)
        .register(router, openapi);

    // The tenant-override middleware is now stateless (`from_fn`): it reads
    // the host-injected `SecurityContext` and rebuilds it when an
    // `X-Insight-Tenant-Id` override header is present. It carries no
    // `AppState` / auth-trait state.
    let api = router.layer(from_fn(auth::tenant_middleware));

    // Health probe — registered on a SEPARATE router merged *after* the
    // tenant middleware, so it stays off the tenant-override path.
    // Kubernetes liveness/readiness probes hit `/health` directly on the pod
    // (no gateway hop, no `X-Insight-Tenant-Id` header), so it must answer
    // without tenant resolution.
    let health = OperationBuilder::get("/health")
        .operation_id("analytics_api.health")
        .summary("Liveness/readiness probe")
        .public()
        .json_response(StatusCode::OK, "Service healthy")
        .handler(handlers::health)
        .register(Router::new(), openapi);

    // Attach the shared state via an Extension layer (route state stays `()`),
    // then merge the public health router.
    api.merge(health).layer(Extension(state))
}
