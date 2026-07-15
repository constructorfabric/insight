//! HTTP API layer: shared state, route table (via `OperationBuilder`), and the
//! subrequest/JWKS contract the nginx gateway consumes (R8).

pub mod error;
pub mod handlers;

use std::sync::Arc;

use axum::http::StatusCode;
use axum::{Extension, Router};
use toolkit::api::{OpenApiRegistry, OperationBuilder};

use crate::config::AuthenticatorConfig;
use crate::identity::PersonResolver;
use crate::jwt::KeyStore;
use crate::oidc::OidcClient;
use crate::service_token::ServiceRegistry;
use crate::session::SessionManager;

/// Shared application state, attached to every route as an `Extension` (main
/// listener) and shared with the service-token listener via `Arc`.
pub struct AppState {
    pub cfg: AuthenticatorConfig,
    pub sessions: SessionManager,
    pub keystore: Arc<KeyStore>,
    pub oidc: OidcClient,
    pub resolver: Arc<dyn PersonResolver>,
    /// Parsed service-token registry (DD-AUTH-05); used by the token listener.
    pub service_registry: ServiceRegistry,
}

/// Register the authenticator routes onto the host router. The `Extension`
/// layer scopes `Arc<AppState>` to these routes (leaving the host's
/// `/health`, `/openapi.json`, `/docs` untouched).
pub fn register_routes(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let api = build_operations(Router::new(), openapi).layer(Extension(state));
    host_router.merge(api)
}

/// Declare every operation through the toolkit's `OperationBuilder` so each
/// lands in the generated OpenAPI (the machine-checkable subrequest contract),
/// grouped by surface. All step-04 endpoints are `.public()` — the credential
/// is the session cookie, checked inside the handler.
fn build_operations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = register_auth_routes(router, openapi);
    let router = register_internal_routes(router, openapi);
    register_well_known_routes(router, openapi)
}

/// The browser-facing `/auth/*` surface (proxied plainly by the gateway).
fn register_auth_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router = router;

    router = OperationBuilder::get("/auth/login")
        .operation_id("authenticator.login")
        .summary("Start the OIDC code+PKCE login flow")
        .tag("auth")
        .public()
        .no_content_response(StatusCode::FOUND, "Redirect to the IdP authorize endpoint")
        .handler(handlers::login)
        .register(router, openapi);

    router = OperationBuilder::get("/auth/callback")
        .operation_id("authenticator.callback")
        .summary("Complete login: exchange the code and set the session cookie")
        .tag("auth")
        .public()
        .no_content_response(
            StatusCode::FOUND,
            "Redirect to the SPA with the session cookie set",
        )
        .handler(handlers::callback)
        .register(router, openapi);

    router = OperationBuilder::get("/auth/me")
        .operation_id("authenticator.me")
        .summary("Current session summary for the SPA")
        .tag("auth")
        .public()
        .text_response(StatusCode::OK, "Session summary", "application/json")
        .error_401(openapi)
        .handler(handlers::me)
        .register(router, openapi);

    OperationBuilder::post("/auth/logout")
        .operation_id("authenticator.logout")
        .summary("Revoke the session, clear the cookie, return the RP-logout URL")
        .tag("auth")
        .public()
        .text_response(StatusCode::OK, "RP-logout URL", "application/json")
        .handler(handlers::logout)
        .register(router, openapi)
}

/// The gateway-facing `/internal/*` surface (the `auth_request` target).
fn register_internal_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::get("/internal/authz")
        .operation_id("authenticator.authz")
        .summary("Exchange the session cookie for the linked gateway JWT")
        .tag("internal")
        .public()
        .no_content_response(StatusCode::OK, "JWT attached via X-Gateway-Jwt")
        .error_401(openapi)
        .handler(handlers::authz)
        .register(router, openapi)
}

/// The public JWKS at `/.well-known/jwks.json` (downstream JWT verification).
fn register_well_known_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::get("/.well-known/jwks.json")
        .operation_id("authenticator.jwks")
        .summary("Public JWKS for gateway-JWT verification")
        .tag("internal")
        .public()
        .text_response(StatusCode::OK, "JWKS document", "application/json")
        .handler(handlers::jwks)
        .register(router, openapi)
}
