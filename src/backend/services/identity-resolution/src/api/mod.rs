//! HTTP API layer — shared state, route table, extractors.

pub(crate) mod canonical_json;
pub mod error;
mod handlers;

use std::sync::Arc;

use axum::Extension;
use axum::Router;
use axum::http::StatusCode;
use sea_orm::DatabaseConnection;
use toolkit::api::{OpenApiRegistry, OperationBuilder};

use crate::config::GearConfig;
use crate::domain::profile;

/// Shared application state, injected into handlers via `Extension`.
#[derive(Clone)]
pub struct AppState {
    /// MariaDB connection pool (SeaORM) — reads `persons` / `account_person_map`.
    pub db: DatabaseConnection,
    /// Gear config (e.g. `org_chart_source_type` for parent/supervisor lookup).
    pub config: GearConfig,
}

/// Mount the identity-resolution routes onto the host's router.
///
/// Builds our endpoints on a fresh sub-router (so the `AppState` extension
/// scopes to our routes, not the host's `/health`/`/docs`), then merges it into
/// the host router. Gateway-JWT identity is enforced entirely by the host authn
/// pipeline: the `oidc-authn-plugin` verifies the ES256 gateway JWT and maps its
/// claims — including the single signed `tenant_id` -> `subject_tenant_id` — into
/// the request `SecurityContext` (`NGINX_BFF` R1). No bespoke tenant layer.
pub fn register_routes(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let api = build_operations(Router::new(), openapi).layer(Extension(state));

    host_router.merge(api)
}

/// Declare each operation via the toolkit `OperationBuilder` (records the route
/// + its OpenAPI spec + auth/error metadata).
fn build_operations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post("/v1/profiles")
        .operation_id("identity_resolution.profiles.resolve")
        .summary("Resolve a profile by email or source-native id")
        .authenticated()
        .no_license_required()
        .json_request::<profile::ResolveProfileRequest>(openapi, "Identity to resolve")
        .json_response_with_schema::<profile::ProfileResponse>(
            openapi,
            StatusCode::OK,
            "Resolved person",
        )
        .standard_errors(openapi)
        .handler(handlers::resolve_profile)
        .register(router, openapi);

    // Deprecated: successor is POST /v1/profiles. Kept for existing callers
    // (authenticator, analytics) until they migrate; emits RFC 8594 headers.
    OperationBuilder::get("/v1/persons/{email}")
        .operation_id("identity_resolution.persons.get")
        .summary("Resolve a person by email (deprecated; use POST /v1/profiles)")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<profile::PersonResponse>(
            openapi,
            StatusCode::OK,
            "Resolved person",
        )
        .standard_errors(openapi)
        .handler(handlers::get_person_by_email)
        .register(router, openapi)
}
