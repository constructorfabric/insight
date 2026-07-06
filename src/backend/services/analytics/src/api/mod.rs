//! HTTP API layer — routes and handlers.

pub(crate) mod admin;
pub(crate) mod canonical_json;
mod catalog;
pub(crate) mod error;
mod handlers;
mod metric_results;

#[cfg(test)]
mod tenant_resolution_tests;

#[cfg(test)]
mod http_live_tests;

#[cfg(test)]
mod openapi_tests;

use axum::http::StatusCode;
use axum::middleware::from_fn;
use axum::{Extension, Router};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use toolkit::api::{OpenApiInfo, OpenApiRegistry, OpenApiRegistryImpl, OperationBuilder};

use crate::auth;
use crate::config::GearConfig;
use crate::domain::admin_threshold::AdminThresholdService;
use crate::domain::admin_threshold::dto as admin_dto;
use crate::domain::auth::TenantAuthorization;
use crate::domain::catalog::CatalogReader;
use crate::domain::catalog::response as catalog_response;
use crate::domain::metric;
use crate::domain::query;
use crate::domain::schema_validator::SchemaValidator;
use crate::domain::threshold;
use crate::infra::identity::{IdentityClient, Person};

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

/// Register all analytics routes onto the host's stateless router.
///
/// Builds the analytics endpoints on a fresh sub-router (via
/// [`build_operations`]) so the tenant-override middleware + `AppState`
/// extension scope to the analytics gear's routes only — not the host's `/health`,
/// `/healthz`, `/openapi.json`, `/docs` — then merges it into the host router.
///
/// The shared `Arc<AppState>` is attached via `router.layer(Extension(state))`
/// and the per-request tenant override via `router.layer(from_fn(tenant_middleware))`.
pub fn register_routes(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let api = build_operations(Router::new(), openapi)
        .layer(from_fn(auth::tenant_middleware))
        .layer(Extension(state));

    host_router.merge(api)
}

/// `OpenAPI` document metadata — the stable API-contract identity baked into
/// the committed `docs/components/backend/analytics/openapi.json` and the
/// spec the offline `analytics openapi` subcommand emits. `version` is the
/// API-contract version (deliberately not `CARGO_PKG_VERSION`), so the drift
/// gate fires only on real route/schema changes, not release bumps.
fn openapi_info() -> OpenApiInfo {
    OpenApiInfo {
        title: "Analytics API".to_owned(),
        version: "1.0.0".to_owned(),
        description: Some(
            "Read-only query service over predefined ClickHouse metrics. Admins \
             define metrics (named SQL queries) in MariaDB; the frontend queries \
             them by UUID with OData-style filtering. The API Gateway mounts this \
             service at /api/analytics."
                .to_owned(),
        ),
        servers: Vec::new(),
    }
}

/// Declare every analytics operation on a **stateless** router.
///
/// Routes are declared through the toolkit's [`OperationBuilder`], so each
/// endpoint records an `OpenAPI` `OperationSpec` plus auth/license metadata in
/// the host-provided `openapi` registry (the gears-rust idiom). Handlers take
/// `Extension<Arc<AppState>>` (state supplied by the caller's layer), so this
/// registers routes without touching any backend — which also makes the full
/// route table unit-testable without constructing an `AppState`/DB.
///
/// `OperationBuilder::register` merges method routers per path, so the
/// shared-path endpoints (`/v1/metrics`, `/v1/admin/metric-thresholds*`) are
/// registered as independent operations.
// One `OperationBuilder` chain per endpoint makes this a long-but-flat route
// table; splitting it further would only obscure the 1:1 route↔handler map.
#[allow(clippy::too_many_lines)]
fn build_operations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router: Router = router;

    // Metric CRUD
    router = OperationBuilder::get("/v1/metrics")
        .operation_id("analytics_api.metrics.list")
        .summary("List metrics")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<metric::MetricListResponse>(
            openapi,
            StatusCode::OK,
            "List of metrics",
        )
        .standard_errors(openapi)
        .handler(handlers::list_metrics)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/metrics")
        .operation_id("analytics_api.metrics.create")
        .summary("Create a metric")
        .authenticated()
        .no_license_required()
        .json_request::<metric::CreateMetricRequest>(openapi, "Metric to create")
        .json_response_with_schema::<metric::Metric>(openapi, StatusCode::CREATED, "Created metric")
        .standard_errors(openapi)
        .handler(handlers::create_metric)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/metrics/{id}")
        .operation_id("analytics_api.metrics.get")
        .summary("Get a metric by id")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<metric::Metric>(openapi, StatusCode::OK, "Metric")
        .standard_errors(openapi)
        .handler(handlers::get_metric)
        .register(router, openapi);

    router = OperationBuilder::put("/v1/metrics/{id}")
        .operation_id("analytics_api.metrics.update")
        .summary("Update a metric")
        .authenticated()
        .no_license_required()
        .json_request::<metric::UpdateMetricRequest>(openapi, "Metric fields to update")
        .json_response_with_schema::<metric::Metric>(openapi, StatusCode::OK, "Updated metric")
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
        .json_request::<query::QueryRequest>(openapi, "OData-style query parameters")
        .json_response_with_schema::<query::QueryResponse>(openapi, StatusCode::OK, "Query result")
        .standard_errors(openapi)
        .handler(handlers::query_metric)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/metrics/queries")
        .operation_id("analytics_api.metrics.query_batch")
        .summary("Query metrics in batch")
        .authenticated()
        .no_license_required()
        .json_request::<query::BatchQueryRequest>(openapi, "Batch of per-metric queries")
        .json_response_with_schema::<query::BatchQueryResponse>(
            openapi,
            StatusCode::OK,
            "Batch query result",
        )
        .standard_errors(openapi)
        .handler(handlers::query_metrics_batch)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/metric-results")
        .operation_id("analytics_api.metric_results.create")
        .summary("Compute metric results")
        .authenticated()
        .no_license_required()
        .json_response(StatusCode::OK, "Metric results")
        .standard_errors(openapi)
        .handler(metric_results::query_metric_results)
        .register(router, openapi);

    // Thresholds (legacy)
    router = OperationBuilder::get("/v1/metrics/{id}/thresholds")
        .operation_id("analytics_api.thresholds.list")
        .summary("List thresholds for a metric")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<threshold::ThresholdListResponse>(
            openapi,
            StatusCode::OK,
            "List of thresholds",
        )
        .standard_errors(openapi)
        .handler(handlers::list_thresholds)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/metrics/{id}/thresholds")
        .operation_id("analytics_api.thresholds.create")
        .summary("Create a threshold for a metric")
        .authenticated()
        .no_license_required()
        .json_request::<threshold::CreateThresholdRequest>(openapi, "Threshold to create")
        .json_response_with_schema::<threshold::Threshold>(
            openapi,
            StatusCode::CREATED,
            "Created threshold",
        )
        .standard_errors(openapi)
        .handler(handlers::create_threshold)
        .register(router, openapi);

    router = OperationBuilder::put("/v1/metrics/{id}/thresholds/{tid}")
        .operation_id("analytics_api.thresholds.update")
        .summary("Update a threshold")
        .authenticated()
        .no_license_required()
        .json_request::<threshold::UpdateThresholdRequest>(openapi, "Threshold fields to update")
        .json_response_with_schema::<threshold::Threshold>(
            openapi,
            StatusCode::OK,
            "Updated threshold",
        )
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
        .json_response_with_schema::<Person>(openapi, StatusCode::OK, "Person")
        .standard_errors(openapi)
        .handler(handlers::get_person)
        .register(router, openapi);

    // Column catalog
    router = OperationBuilder::get("/v1/columns")
        .operation_id("analytics_api.columns.list")
        .summary("List queryable columns")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<metric::ColumnListResponse>(
            openapi,
            StatusCode::OK,
            "List of columns",
        )
        .standard_errors(openapi)
        .handler(handlers::list_columns)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/columns/{table}")
        .operation_id("analytics_api.columns.list_for_table")
        .summary("List queryable columns for a table")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<metric::ColumnListResponse>(
            openapi,
            StatusCode::OK,
            "List of columns",
        )
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
        .json_request::<catalog_response::GetMetricsRequest>(
            openapi,
            "Catalog read request context (role, team)",
        )
        .json_response_with_schema::<catalog_response::CatalogResponse>(
            openapi,
            StatusCode::OK,
            "Resolved metric catalog",
        )
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
        .json_response_with_schema::<admin_dto::ListResponse>(
            openapi,
            StatusCode::OK,
            "List of metric thresholds",
        )
        .standard_errors(openapi)
        .handler(admin::list)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/admin/metric-thresholds")
        .operation_id("analytics_api.admin.thresholds.create")
        .summary("Create an admin metric threshold")
        .authenticated()
        .no_license_required()
        .json_request::<admin_dto::CreateRequest>(openapi, "Metric threshold to create")
        .json_response_with_schema::<admin_dto::ThresholdView>(
            openapi,
            StatusCode::CREATED,
            "Created metric threshold",
        )
        .standard_errors(openapi)
        .handler(admin::create)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/admin/metric-thresholds/{id}")
        .operation_id("analytics_api.admin.thresholds.get")
        .summary("Get an admin metric threshold by id")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<admin_dto::ThresholdView>(
            openapi,
            StatusCode::OK,
            "Metric threshold",
        )
        .standard_errors(openapi)
        .handler(admin::get_one)
        .register(router, openapi);

    router = OperationBuilder::put("/v1/admin/metric-thresholds/{id}")
        .operation_id("analytics_api.admin.thresholds.update")
        .summary("Update an admin metric threshold")
        .authenticated()
        .no_license_required()
        .json_request::<admin_dto::UpdateRequest>(openapi, "Metric threshold fields to update")
        .json_response_with_schema::<admin_dto::ThresholdView>(
            openapi,
            StatusCode::OK,
            "Updated metric threshold",
        )
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

    // `/health` + `/healthz` are provided by the api-gateway host gear (its
    // `rest_prepare`), so we must NOT register them here — doing so panics with
    // "Overlapping method route". State + the (stateless `from_fn`) tenant
    // middleware are layered by `register_routes`, not here.
    router
}

/// Build the analytics `OpenAPI` document **offline** — no `AppState`, DB,
/// or HTTP listener. Backs the `analytics openapi` subcommand (committed-doc
/// regeneration + drift gate), reusing the exact `build_operations` route table
/// the live gear serves, so the two can never diverge.
pub fn openapi_document() -> anyhow::Result<utoipa::openapi::OpenApi> {
    let openapi = OpenApiRegistryImpl::new();
    let _ = build_operations(Router::new(), &openapi);
    openapi
        .build_openapi(&openapi_info())
        .map_err(|e| anyhow::anyhow!("failed to build analytics OpenAPI document: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use toolkit::api::OpenApiRegistryImpl;

    /// Exercises the full route table + `OpenAPI` registration with no `AppState`
    /// or DB: handlers are only *registered* (via `Extension` extractors), never
    /// invoked. Guards against overlapping-route panics / bad `OperationBuilder`
    /// state, and records every `OperationSpec` in the registry.
    #[test]
    fn build_operations_registers_the_full_table_without_state() {
        let openapi = OpenApiRegistryImpl::new();
        let _router: Router = build_operations(Router::new(), &openapi);
    }
}
