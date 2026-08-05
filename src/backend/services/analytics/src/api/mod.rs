//! HTTP API layer — routes and handlers.

pub(crate) mod error;
mod metric_definitions;
mod metric_drilldown;
mod metric_results;
mod saved_queries;

#[cfg(test)]
mod http_live_tests;

#[cfg(test)]
mod openapi_tests;

use axum::http::StatusCode;
use axum::{Extension, Router};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use toolkit::api::{
    OpenApiInfo, OpenApiRegistry, OpenApiRegistryImpl, OperationBuilder, ResponseSpec,
};
use utoipa::openapi::RefOr;
use utoipa::openapi::content::ContentBuilder;
use utoipa::openapi::header::HeaderBuilder;
use utoipa::openapi::schema::{
    KnownFormat, ObjectBuilder, Schema, SchemaFormat, SchemaType, Type as OpenApiType,
};

use crate::config::GearConfig;
use crate::domain::metric_definitions::listing as metric_definitions_listing;
use crate::domain::saved_query;
use crate::infra::identity::IdentityClient;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub ch: insight_clickhouse::Client,
    pub identity: IdentityClient,
    #[allow(dead_code)] // will be used for runtime config access (rate limits, feature flags)
    pub config: GearConfig,
}

pub(crate) fn forwarded_authorization(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
}

/// Register all analytics routes onto the host's stateless router.
///
/// Builds the analytics endpoints on a fresh sub-router (via
/// [`build_operations`]) so the tenant-override middleware + `AppState`
/// extension scope to the analytics gear's routes only — not the host's `/health`,
/// `/healthz`, `/openapi.json`, `/docs` — then merges it into the host router.
///
/// The shared `Arc<AppState>` is attached via `router.layer(Extension(state))`.
/// Gateway-JWT identity is enforced entirely by the host authn pipeline: the
/// `oidc-authn-plugin` verifies the ES256 gateway JWT (signature / `iss` /
/// `aud` / `exp`) against the authenticator's JWKS and maps its claims to the
/// request `SecurityContext` via configured `claim_mapping` (`sub` →
/// `subject_id`, `tenant_id` → `subject_tenant_id`, `roles` → `token_scopes`;
/// `subject_tenant_id` is required). No bespoke mapping layer here
/// (`NGINX_BFF` R1 / G2).
pub fn register_routes(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let api = build_operations(Router::new(), openapi).layer(Extension(state));

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
            "Read-only query service over ClickHouse. Serves declarative metric \
             definitions, computed metric results and their evidence, plus \
             tenant-authored saved queries. The API Gateway mounts this service \
             at /api/analytics."
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
/// `OperationBuilder::register` merges method routers per path, so endpoints
/// sharing a path across methods are registered as independent operations.
// One `OperationBuilder` chain per endpoint makes this a long-but-flat route
// table; splitting it further would only obscure the 1:1 route↔handler map.
#[allow(clippy::too_many_lines)]
fn build_operations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router: Router = router;

    router = OperationBuilder::post("/v1/metric-results")
        .operation_id("analytics_api.metric_results.create")
        .summary("Compute metric results")
        .authenticated()
        .no_license_required()
        .json_request::<crate::domain::metric_results::MetricResultsRequest>(
            openapi,
            "Metric result request",
        )
        .json_response_with_schema::<crate::domain::metric_results::MetricResultsResponse>(
            openapi,
            StatusCode::OK,
            "Metric results",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_415(openapi)
        .error_500(openapi)
        .handler(metric_results::query_metric_results)
        .register(router, openapi);

    // Saved-query CRUD + run (#1965) — the presentation-layer "Data Analytics"
    // surface. CRUD is service-DB metadata; only `/run` reaches ClickHouse.
    router = OperationBuilder::get("/v1/queries")
        .operation_id("analytics_api.queries.list")
        .summary("List saved queries")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<saved_query::SavedQueryListResponse>(
            openapi,
            StatusCode::OK,
            "List of saved queries",
        )
        .standard_errors(openapi)
        .handler(saved_queries::list_saved_queries)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/queries")
        .operation_id("analytics_api.queries.create")
        .summary("Create a saved query")
        .authenticated()
        .no_license_required()
        .json_request::<saved_query::CreateSavedQueryRequest>(openapi, "Saved query to create")
        .json_response_with_schema::<saved_query::SavedQuery>(
            openapi,
            StatusCode::CREATED,
            "Created saved query",
        )
        .standard_errors(openapi)
        .handler(saved_queries::create_saved_query)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/queries/{id}")
        .operation_id("analytics_api.queries.get")
        .summary("Get a saved query by id")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<saved_query::SavedQuery>(
            openapi,
            StatusCode::OK,
            "Saved query",
        )
        .standard_errors(openapi)
        .handler(saved_queries::get_saved_query)
        .register(router, openapi);

    router = OperationBuilder::put("/v1/queries/{id}")
        .operation_id("analytics_api.queries.update")
        .summary("Update a saved query")
        .authenticated()
        .no_license_required()
        .json_request::<saved_query::UpdateSavedQueryRequest>(
            openapi,
            "Saved query fields to update",
        )
        .json_response_with_schema::<saved_query::SavedQuery>(
            openapi,
            StatusCode::OK,
            "Updated saved query",
        )
        .standard_errors(openapi)
        .handler(saved_queries::update_saved_query)
        .register(router, openapi);

    router = OperationBuilder::delete("/v1/queries/{id}")
        .operation_id("analytics_api.queries.delete")
        .summary("Delete a saved query")
        .authenticated()
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Saved query deleted")
        .standard_errors(openapi)
        .handler(saved_queries::delete_saved_query)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/queries/{id}/run")
        .operation_id("analytics_api.queries.run")
        .summary("Run a saved query")
        .authenticated()
        .no_license_required()
        .json_request::<saved_query::RunSavedQueryRequest>(
            openapi,
            "Optional named parameters (`period`); `tenant` is always injected from context",
        )
        .request_optional()
        .json_response_with_schema::<saved_query::RunResponse>(
            openapi,
            StatusCode::OK,
            "Query result rows",
        )
        .standard_errors(openapi)
        .handler(saved_queries::run_saved_query)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/metric-drilldown")
        .operation_id("analytics_api.metric_drilldown.create")
        .summary("List metric evidence")
        .authenticated()
        .no_license_required()
        .json_request::<crate::domain::metric_drilldown::MetricDrilldownRequest>(
            openapi,
            "Metric evidence selection",
        )
        .json_response_with_schema::<crate::domain::metric_drilldown::MetricDrilldownResponse>(
            openapi,
            StatusCode::OK,
            "Metric evidence",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_404(openapi)
        .error_415(openapi)
        .error_429(openapi)
        .error_500(openapi)
        .handler(metric_drilldown::query_metric_drilldown)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/metric-drilldown/export")
        .operation_id("analytics_api.metric_drilldown.export")
        .summary("Export metric evidence")
        .authenticated()
        .no_license_required()
        .json_request::<crate::domain::metric_drilldown::MetricDrilldownExportRequest>(
            openapi,
            "Metric evidence export selection",
        )
        .response(ResponseSpec {
            status: StatusCode::OK.as_u16(),
            content_type: "text/csv",
            description: "Complete metric evidence export".to_owned(),
            schema_name: None,
        })
        .standard_errors(openapi)
        .handler(metric_drilldown::export_metric_drilldown)
        .register(router, openapi);

    // Unified metric definitions listing — display fields only, tenant
    // scope resolved server-side from the session. GET is safe here: no
    // request-context fields exist to leak into access logs.
    router = OperationBuilder::get("/v1/metric-definitions")
        .operation_id("analytics_api.metric_definitions.list")
        .summary("List unified metric definitions")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<metric_definitions_listing::MetricDefinitionListResponse>(
            openapi,
            StatusCode::OK,
            "Metric definitions",
        )
        .error_401(openapi)
        .error_500(openapi)
        .handler(metric_definitions::list_metric_definitions)
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
    let mut document = openapi
        .build_openapi(&openapi_info())
        .map_err(|e| anyhow::anyhow!("failed to build analytics OpenAPI document: {e}"))?;
    let response = document
        .paths
        .paths
        .get_mut("/v1/metric-drilldown/export")
        .and_then(|path| path.post.as_mut())
        .and_then(|operation| operation.responses.responses.get_mut("200"))
        .ok_or_else(|| anyhow::anyhow!("metric drilldown export response is missing"))?;
    let RefOr::T(response) = response else {
        return Err(anyhow::anyhow!(
            "metric drilldown export response must be inline"
        ));
    };
    let schema = Schema::Object(
        ObjectBuilder::new()
            .schema_type(SchemaType::Type(OpenApiType::String))
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Binary)))
            .build(),
    );
    for media_type in [
        "text/csv",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ] {
        response.content.insert(
            media_type.to_owned(),
            ContentBuilder::new().schema(Some(schema.clone())).build(),
        );
    }
    response.headers.insert(
        "Content-Disposition".to_owned(),
        HeaderBuilder::new()
            .schema(ObjectBuilder::new().schema_type(OpenApiType::String))
            .description(Some("Attachment filename"))
            .build(),
    );
    Ok(document)
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

    #[test]
    fn export_response_advertises_both_file_media_types_and_the_filename_header() {
        let document =
            openapi_document().unwrap_or_else(|error| panic!("document must build: {error}"));
        let response = document
            .paths
            .paths
            .get("/v1/metric-drilldown/export")
            .and_then(|path| path.post.as_ref())
            .and_then(|operation| operation.responses.responses.get("200"))
            .unwrap_or_else(|| panic!("export 200 response must be registered"));
        let RefOr::T(response) = response else {
            panic!("export response must be inline, not a $ref");
        };

        for media_type in [
            "text/csv",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ] {
            assert!(
                response.content.contains_key(media_type),
                "export must advertise {media_type}"
            );
        }
        assert!(
            response.headers.contains_key("Content-Disposition"),
            "export must advertise the attachment filename header"
        );
    }
}
