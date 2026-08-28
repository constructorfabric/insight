//! HTTP API layer — routes and handlers.

pub(crate) mod ai;
mod connector_health;
pub(crate) mod error;
mod feedback;
mod metric_definitions;
mod metric_drilldown;
mod metric_results;
mod metrics;
mod person_names;
mod reports;
mod saved_queries;
pub(crate) mod usage;

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
use toolkit_canonical_errors::CanonicalError;
use utoipa::openapi::RefOr;
use utoipa::openapi::content::ContentBuilder;
use utoipa::openapi::header::HeaderBuilder;
use utoipa::openapi::schema::{
    KnownFormat, ObjectBuilder, Schema, SchemaFormat, SchemaType, Type as OpenApiType,
};

use tokio::sync::Semaphore;

use crate::config::GearConfig;
use crate::domain::ai::dto as ai_dto;
use crate::domain::connector_health as connector_health_domain;
use crate::domain::external_links::ExternalSourceRegistry;
use crate::domain::metric_crud;
use crate::domain::metric_definitions::listing as metric_definitions_listing;
use crate::domain::saved_query;
use crate::infra::anthropic::AnthropicClient;
use crate::infra::identity::IdentityClient;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub ch: insight_clickhouse::Client,
    pub identity: IdentityClient,
    pub anthropic: AnthropicClient,
    /// Caps explain calls in flight in this process.
    pub ai_calls: Arc<Semaphore>,
    pub report_generations: Arc<Semaphore>,
    pub report_artifacts: Arc<Semaphore>,
    pub config: GearConfig,
    pub external_links: ExternalSourceRegistry,
}

pub(crate) fn forwarded_authorization(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
}

// SAFETY: an identity that is absent or unreachable is a server error, never a
// permit.
pub(crate) async fn is_admin_caller(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<bool, CanonicalError> {
    if !state.identity.is_configured() {
        tracing::error!("identity service is not configured; admin access cannot be verified");
        return Err(CanonicalError::internal("failed to verify caller permissions").create());
    }

    state
        .identity
        .is_admin(forwarded_authorization(headers))
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "admin role check failed");
            CanonicalError::internal("failed to verify caller permissions").create()
        })
}

/// What a refused admin surface says. The builder is generic over its own
/// resource, so each caller constructs the refusal in its namespace and this
/// gate decides only whether to raise it.
pub(crate) const ADMIN_ONLY: &str = "admin role required for this operation";

pub(crate) async fn require_admin(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    denied: fn() -> CanonicalError,
) -> Result<(), CanonicalError> {
    if is_admin_caller(state, headers).await? {
        return Ok(());
    }
    Err(denied())
}

/// Clips to a CHARACTER budget: a byte slice would split a multi-byte value.
pub(crate) fn clip(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
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
pub(crate) fn build_operations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router: Router = router;

    // Usage monitoring (#2573). Ingest is open to any signed-in caller — it is
    // the SPA's beacon; the read model is admin-gated inside the handler.
    router = OperationBuilder::post("/v1/usage/events")
        .operation_id("analytics_api.usage.ingest")
        .summary("Record usage events")
        .authenticated()
        .no_license_required()
        .json_request::<usage::UsageIngestRequest>(openapi, "Telemetry SDK records")
        .no_content_response(StatusCode::NO_CONTENT, "Accepted")
        .error_401(openapi)
        .error_415(openapi)
        .handler(usage::ingest_usage_events)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/usage/config")
        .operation_id("analytics_api.usage.config")
        .summary("Whether this instance records usage")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<usage::UsageConfigResponse>(
            openapi,
            StatusCode::OK,
            "Usage collection state",
        )
        .standard_errors(openapi)
        .handler(usage::get_usage_config)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/usage/summary")
        .operation_id("analytics_api.usage.summary")
        .summary("Usage summary for a date range")
        .authenticated()
        .no_license_required()
        .query_param_typed("since", false, "Inclusive first day, YYYY-MM-DD", "string")
        .query_param_typed("until", false, "Inclusive last day, YYYY-MM-DD", "string")
        .json_response_with_schema::<usage::UsageSummaryResponse>(
            openapi,
            StatusCode::OK,
            "Usage summary",
        )
        .standard_errors(openapi)
        .handler(usage::get_usage_summary)
        .register(router, openapi);

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
        .json_response_with_schema::<connector_health_domain::SyncHistoryResponse>(
            openapi,
            StatusCode::OK,
            "A bounded window of recorded syncs",
        )
        .standard_errors(openapi)
        .handler(connector_health::get_connector_syncs)
        .register(router, openapi);

    // Sending is open to any signed-in caller; the listing is admin-gated
    // inside the handler, not here.
    router = OperationBuilder::post("/v1/feedback")
        .operation_id("analytics_api.feedback.submit")
        .summary("Send product feedback")
        .authenticated()
        .no_license_required()
        .json_request::<feedback::FeedbackRequest>(openapi, "A feedback submission")
        .no_content_response(StatusCode::NO_CONTENT, "Recorded")
        // Only what a submission can actually answer: it addresses no resource,
        // so the standard bundle's 404/409/429 would promise cases with no path.
        .error_400(openapi)
        .error_401(openapi)
        .error_415(openapi)
        .error_500(openapi)
        .handler(feedback::submit_feedback)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/feedback")
        .operation_id("analytics_api.feedback.list")
        .summary("Feedback sent in a date range")
        .authenticated()
        .no_license_required()
        .query_param_typed("since", false, "Inclusive first day, YYYY-MM-DD", "string")
        .query_param_typed("until", false, "Inclusive last day, YYYY-MM-DD", "string")
        .json_response_with_schema::<feedback::FeedbackListResponse>(
            openapi,
            StatusCode::OK,
            "Feedback entries, newest first",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .handler(feedback::list_feedback)
        .register(router, openapi);

    // AI assist. `config` answers on every stand — "off" is the answer the SPA
    // needs; the rest 404 while the stand switch is off.
    router = OperationBuilder::get("/v1/ai/config")
        .operation_id("analytics_api.ai.config")
        .summary("Whether this instance explains metrics with AI")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<ai_dto::AiConfigResponse>(
            openapi,
            StatusCode::OK,
            "AI assist state",
        )
        .standard_errors(openapi)
        .handler(ai::get_ai_config)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/ai/credentials")
        .operation_id("analytics_api.ai.credentials.get")
        .summary("Whether the caller has an Anthropic key stored")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<ai_dto::AiCredentialResponse>(
            openapi,
            StatusCode::OK,
            "Stored-key state",
        )
        .standard_errors(openapi)
        .handler(ai::credentials::get_credential)
        .register(router, openapi);

    router = OperationBuilder::put("/v1/ai/credentials")
        .operation_id("analytics_api.ai.credentials.put")
        .summary("Store or replace the caller's Anthropic key")
        .authenticated()
        .no_license_required()
        .json_request::<ai_dto::PutCredentialRequest>(openapi, "The key to store")
        .json_response_with_schema::<ai_dto::AiCredentialResponse>(
            openapi,
            StatusCode::OK,
            "Stored-key state",
        )
        .standard_errors(openapi)
        .handler(ai::credentials::put_credential)
        .register(router, openapi);

    router = OperationBuilder::delete("/v1/ai/credentials")
        .operation_id("analytics_api.ai.credentials.delete")
        .summary("Forget the caller's Anthropic key")
        .authenticated()
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Key removed")
        .standard_errors(openapi)
        .handler(ai::credentials::delete_credential)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/ai/settings")
        .operation_id("analytics_api.ai.settings.get")
        .summary("The system prompt in force for this tenant")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<ai_dto::AiSettingsResponse>(
            openapi,
            StatusCode::OK,
            "System prompt",
        )
        .standard_errors(openapi)
        .handler(ai::settings::get_settings)
        .register(router, openapi);

    router = OperationBuilder::put("/v1/ai/settings")
        .operation_id("analytics_api.ai.settings.put")
        .summary("Replace this tenant's system prompt")
        .authenticated()
        .no_license_required()
        .json_request::<ai_dto::PutSettingsRequest>(openapi, "The prompt to store")
        .json_response_with_schema::<ai_dto::AiSettingsResponse>(
            openapi,
            StatusCode::OK,
            "System prompt",
        )
        .standard_errors(openapi)
        .handler(ai::settings::put_settings)
        .register(router, openapi);

    router = OperationBuilder::delete("/v1/ai/settings")
        .operation_id("analytics_api.ai.settings.reset")
        .summary("Restore the shipped system prompt")
        .authenticated()
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Prompt reset")
        .standard_errors(openapi)
        .handler(ai::settings::reset_settings)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/ai/context")
        .operation_id("analytics_api.ai.context.list")
        .summary("Context entries the caller's explanations read")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<ai_dto::ContextListResponse>(
            openapi,
            StatusCode::OK,
            "Context entries",
        )
        .standard_errors(openapi)
        .handler(ai::context::list_context)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/ai/context")
        .operation_id("analytics_api.ai.context.create")
        .summary("Add a context entry")
        .authenticated()
        .no_license_required()
        .json_request::<ai_dto::CreateContextRequest>(openapi, "Entry to add")
        .json_response_with_schema::<ai_dto::ContextEntryResponse>(
            openapi,
            StatusCode::CREATED,
            "Created entry",
        )
        .standard_errors(openapi)
        .handler(ai::context::create_context)
        .register(router, openapi);

    router = OperationBuilder::patch("/v1/ai/context/{id}")
        .operation_id("analytics_api.ai.context.update")
        .summary("Edit a context entry")
        .authenticated()
        .no_license_required()
        .json_request::<ai_dto::UpdateContextRequest>(openapi, "Fields to change")
        .json_response_with_schema::<ai_dto::ContextEntryResponse>(
            openapi,
            StatusCode::OK,
            "Updated entry",
        )
        .standard_errors(openapi)
        .handler(ai::context::update_context)
        .register(router, openapi);

    router = OperationBuilder::delete("/v1/ai/context/{id}")
        .operation_id("analytics_api.ai.context.delete")
        .summary("Remove a context entry")
        .authenticated()
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Entry removed")
        .standard_errors(openapi)
        .handler(ai::context::delete_context)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/ai/explain")
        .operation_id("analytics_api.ai.explain")
        .summary("Explain one metric reading")
        .authenticated()
        .no_license_required()
        .json_request::<ai::explain::ExplainRequest>(openapi, "The tile to explain")
        .json_response_with_schema::<ai::explain::ExplainResponse>(
            openapi,
            StatusCode::OK,
            "The explanation",
        )
        .standard_errors(openapi)
        .handler(ai::explain::explain_metric)
        .register(router, openapi);

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

    router = OperationBuilder::post("/v1/reports/preview")
        .operation_id("analytics_api.reports.preview")
        .summary("Preview a metric report")
        .authenticated()
        .no_license_required()
        .json_request::<crate::domain::reports::dto::ReportPreviewRequest>(openapi, "Report recipe")
        .json_response_with_schema::<crate::domain::reports::dto::ReportPreviewResponse>(
            openapi,
            StatusCode::OK,
            "Report preview",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_415(openapi)
        .error_429(openapi)
        .error_500(openapi)
        .handler(reports::preview_report)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/reports/export")
        .operation_id("analytics_api.reports.export")
        .summary("Export a metric report")
        .authenticated()
        .no_license_required()
        .json_request::<crate::domain::reports::dto::ReportExportRequest>(
            openapi,
            "Report export recipe",
        )
        .response(ResponseSpec {
            status: StatusCode::OK.as_u16(),
            content_type: "text/csv",
            description: "Complete metric report export".to_owned(),
            schema_name: None,
        })
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_415(openapi)
        .error_429(openapi)
        .error_500(openapi)
        .handler(reports::export_report)
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
        .error_403(openapi)
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

    // Custom-metric CRUD + export/import — the `origin='custom'` authoring
    // surface. Every route is tenant-scoped and touches custom rows only, so a
    // builtin key is read-only through this API.
    router = OperationBuilder::get("/v1/metrics")
        .operation_id("analytics_api.metrics.list")
        .summary("List custom metrics")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<metric_crud::CustomMetricListResponse>(
            openapi,
            StatusCode::OK,
            "List of custom metrics",
        )
        .standard_errors(openapi)
        .handler(metrics::list_custom_metrics)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/metrics")
        .operation_id("analytics_api.metrics.create")
        .summary("Create a custom metric")
        .authenticated()
        .no_license_required()
        .json_request::<metric_crud::CustomMetric>(openapi, "Custom metric to create")
        .json_response_with_schema::<metric_crud::CustomMetric>(
            openapi,
            StatusCode::CREATED,
            "Created custom metric",
        )
        .standard_errors(openapi)
        .handler(metrics::create_custom_metric_handler)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/metrics/export")
        .operation_id("analytics_api.metrics.export")
        .summary("Export custom metrics")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<metric_crud::ExportCustomMetricsResponse>(
            openapi,
            StatusCode::OK,
            "Portable custom metric graphs",
        )
        .standard_errors(openapi)
        .handler(metrics::export_custom_metrics_handler)
        .register(router, openapi);

    router = OperationBuilder::post("/v1/metrics/import")
        .operation_id("analytics_api.metrics.import")
        .summary("Import custom metrics")
        .authenticated()
        .no_license_required()
        .json_request::<metric_crud::ImportCustomMetricsRequest>(
            openapi,
            "Custom metric graphs to import",
        )
        .json_response_with_schema::<metric_crud::ImportCustomMetricsResponse>(
            openapi,
            StatusCode::OK,
            "Import result",
        )
        .standard_errors(openapi)
        .handler(metrics::import_custom_metrics_handler)
        .register(router, openapi);

    router = OperationBuilder::get("/v1/metrics/{metric_key}")
        .operation_id("analytics_api.metrics.get")
        .summary("Get a custom metric by key")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<metric_crud::CustomMetric>(
            openapi,
            StatusCode::OK,
            "Custom metric",
        )
        .standard_errors(openapi)
        .handler(metrics::get_custom_metric)
        .register(router, openapi);

    router = OperationBuilder::put("/v1/metrics/{metric_key}")
        .operation_id("analytics_api.metrics.update")
        .summary("Update a custom metric")
        .authenticated()
        .no_license_required()
        .json_request::<metric_crud::CustomMetric>(openapi, "Custom metric fields to replace")
        .json_response_with_schema::<metric_crud::CustomMetric>(
            openapi,
            StatusCode::OK,
            "Updated custom metric",
        )
        .standard_errors(openapi)
        .handler(metrics::update_custom_metric_handler)
        .register(router, openapi);

    router = OperationBuilder::delete("/v1/metrics/{metric_key}")
        .operation_id("analytics_api.metrics.delete")
        .summary("Delete a custom metric")
        .authenticated()
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Custom metric deleted")
        .standard_errors(openapi)
        .handler(metrics::delete_custom_metric_handler)
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
    add_file_export_response(&mut document, "/v1/metric-drilldown/export")?;
    add_file_export_response(&mut document, "/v1/reports/export")?;
    Ok(document)
}

fn add_file_export_response(
    document: &mut utoipa::openapi::OpenApi,
    path: &str,
) -> anyhow::Result<()> {
    let response = document
        .paths
        .paths
        .get_mut(path)
        .and_then(|path| path.post.as_mut())
        .and_then(|operation| operation.responses.responses.get_mut("200"))
        .ok_or_else(|| anyhow::anyhow!("file export response is missing for {path}"))?;
    let RefOr::T(response) = response else {
        return Err(anyhow::anyhow!("file export response must be inline"));
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
    Ok(())
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

    #[test]
    fn report_export_response_advertises_both_file_media_types_and_the_filename_header() {
        let document =
            openapi_document().unwrap_or_else(|error| panic!("document must build: {error}"));
        let response = document
            .paths
            .paths
            .get("/v1/reports/export")
            .and_then(|path| path.post.as_ref())
            .and_then(|operation| operation.responses.responses.get("200"))
            .unwrap_or_else(|| panic!("report export 200 response must be registered"));
        let RefOr::T(response) = response else {
            panic!("report export response must be inline, not a $ref");
        };

        for media_type in [
            "text/csv",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ] {
            assert!(
                response.content.contains_key(media_type),
                "report export must advertise {media_type}"
            );
        }
        assert!(
            response.headers.contains_key("Content-Disposition"),
            "report export must advertise the attachment filename header"
        );
    }
}
