//! HTTP API layer: shared state, route table (via `OperationBuilder`), and the
//! subrequest/JWKS contract the nginx gateway consumes (R8).

pub mod error;
pub mod handlers;

use std::sync::Arc;

use axum::http::StatusCode;
use axum::{Extension, Router};
use toolkit::api::{OpenApiInfo, OpenApiRegistry, OpenApiRegistryImpl, OperationBuilder};

use crate::config::AuthenticatorConfig;
use crate::identity::PersonResolver;
use crate::issuers::IssuerSelector;
use crate::jwt::KeyStore;
use crate::service_token::ServiceRegistry;
use crate::session::SessionManager;

/// Shared application state, attached to every route as an `Extension` (main
/// listener) and shared with the service-token listener via `Arc`.
pub struct AppState {
    pub cfg: AuthenticatorConfig,
    pub sessions: SessionManager,
    pub keystore: Arc<KeyStore>,
    /// The configured OIDC issuer(s): host-keyed at `/auth/login`, pinned by
    /// issuer everywhere else (ADR-0003).
    pub oidc: IssuerSelector,
    pub resolver: Arc<dyn PersonResolver>,
    /// Parsed service-token registry (DD-AUTH-05); used by the token listener.
    pub service_registry: ServiceRegistry,
    /// The SDK contract impl (also registered in the `ClientHub`): the admin
    /// revoke-by-user operation goes through it, so the HTTP surface and
    /// in-process consumers (the future permissions service) share one path.
    pub authn_client: Arc<dyn authenticator_sdk::AuthenticatorClientV1>,
    /// Audit publisher (Redpanda; no-op when unconfigured).
    pub audit: crate::audit::AuditEmitter,
}

/// Register the authenticator routes onto the host router. The `Extension`
/// layer scopes `Arc<AppState>` to these routes (leaving the host's
/// `/health`, `/openapi.json`, `/docs` untouched).
pub fn register_routes(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    // CSRF verification wraps the route table (state-changing `/auth/*` only —
    // the middleware filters); the Extension layer runs first so handlers and
    // middleware share the same state.
    let api = build_operations(Router::new(), openapi)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::csrf::middleware,
        ))
        .layer(Extension(state));
    host_router.merge(api)
}

/// Declare every operation through the toolkit's `OperationBuilder` so each
/// lands in the generated OpenAPI (the machine-checkable subrequest contract),
/// grouped by surface. All step-04 endpoints are `.public()` — the credential
/// is the session cookie, checked inside the handler.
fn build_operations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = register_auth_routes(router, openapi);
    let router = register_session_routes(router, openapi);
    let router = register_internal_routes(router, openapi);
    register_well_known_routes(router, openapi)
}

/// The browser-facing `/auth/*` surface (proxied plainly by the gateway).
fn register_auth_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router = router;

    router = OperationBuilder::get("/auth/login")
        .operation_id("authenticator.login")
        .summary("Start the OIDC code+PKCE login flow (the request Host selects the issuer)")
        .tag("auth")
        .public()
        .no_content_response(StatusCode::FOUND, "Redirect to the IdP authorize endpoint")
        .error_403(openapi)
        .handler(handlers::login)
        .register(router, openapi);

    router = OperationBuilder::get("/auth/callback")
        .operation_id("authenticator.callback")
        .summary("Complete login: exchange the code and set the session cookie")
        .tag("auth")
        .public()
        .no_content_response(
            StatusCode::FOUND,
            "Redirect to the SPA: with the session cookie set on success, or \
             with `auth_error=<reason>` (state_expired, idp_error, \
             invalid_callback, exchange_failed, access_denied) on a failed \
             login so the SPA can restart the flow",
        )
        .handler(handlers::callback)
        .register(router, openapi);

    router = OperationBuilder::get("/auth/csrf")
        .operation_id("authenticator.csrf")
        .summary("Issue the CSRF token bound to the current session")
        .tag("auth")
        .public()
        .text_response(StatusCode::OK, "CSRF token", "application/json")
        .error_401(openapi)
        .handler(handlers::csrf)
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

    router = OperationBuilder::post("/auth/refresh")
        .operation_id("authenticator.refresh")
        .summary("Rotate the session cookie and extend the session (grace-tolerant)")
        .tag("auth")
        .public()
        .text_response(
            StatusCode::OK,
            "{expires_at, refresh_at} + re-issued cookie",
            "application/json",
        )
        .error_401(openapi)
        .handler(handlers::refresh)
        .register(router, openapi);

    router = OperationBuilder::post("/auth/oidc/back-channel-logout")
        .operation_id("authenticator.back_channel_logout")
        .summary("Receive IdP back-channel logout tokens (OIDC BCL 1.0)")
        .tag("auth")
        .public()
        .no_content_response(StatusCode::OK, "Logout processed (or idempotent replay)")
        .error_400(openapi)
        .handler(handlers::back_channel_logout)
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

/// The session-management surface (PRD 5.9): list + revoke for the current
/// user, and the gateway-JWT-authenticated admin revoke-by-user variant.
fn register_session_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router = router;

    router = OperationBuilder::get("/auth/sessions")
        .operation_id("authenticator.sessions.list")
        .summary("List the current user's active sessions")
        .tag("auth")
        .public()
        .text_response(StatusCode::OK, "Active sessions", "application/json")
        .error_401(openapi)
        .handler(handlers::sessions_list)
        .register(router, openapi);

    router = OperationBuilder::delete("/auth/sessions/{session_id}")
        .operation_id("authenticator.sessions.revoke")
        .summary("Revoke one of the current user's sessions")
        .tag("auth")
        .public()
        .text_response(StatusCode::OK, "Revocation result", "application/json")
        .error_401(openapi)
        .error_404(openapi)
        .handler(handlers::sessions_revoke_one)
        .register(router, openapi);

    router = OperationBuilder::delete("/auth/sessions")
        .operation_id("authenticator.sessions.revoke_all")
        .summary("Revoke all sessions of the current user (log out everywhere)")
        .tag("auth")
        .public()
        .text_response(StatusCode::OK, "Revocation result", "application/json")
        .error_401(openapi)
        .handler(handlers::sessions_revoke_all)
        .register(router, openapi);

    // Admin/service variant (PRD 5.9): `.authenticated()` — the host authn
    // pipeline verifies a gateway JWT (the authenticator trusts its own tokens
    // exactly like any downstream service, G10) and the handler enforces the
    // authorized role.
    router = OperationBuilder::delete("/auth/admin/users/{person_id}/sessions")
        .operation_id("authenticator.sessions.admin_revoke_by_user")
        .summary("Revoke every session of a user (admin/service, gateway-JWT authenticated)")
        .tag("auth")
        .authenticated()
        .no_license_required()
        .text_response(StatusCode::OK, "Revocation result", "application/json")
        .error_401(openapi)
        .error_403(openapi)
        .handler(handlers::admin_revoke_user_sessions)
        .register(router, openapi);

    router
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

/// The public well-known surface for downstream JWT verification: the OIDC
/// discovery document (`cf-gears-oidc-authn-plugin` resolves the JWKS from it)
/// and the JWKS itself.
fn register_well_known_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/.well-known/openid-configuration")
        .operation_id("authenticator.openid_configuration")
        .summary("OIDC discovery document (issuer + jwks_uri) for downstream verifiers")
        .tag("internal")
        .public()
        .text_response(
            StatusCode::OK,
            "OIDC discovery document",
            "application/json",
        )
        .handler(handlers::openid_configuration)
        .register(router, openapi);

    OperationBuilder::get("/.well-known/jwks.json")
        .operation_id("authenticator.jwks")
        .summary("Public JWKS for gateway-JWT verification")
        .tag("internal")
        .public()
        .text_response(StatusCode::OK, "JWKS document", "application/json")
        .handler(handlers::jwks)
        .register(router, openapi)
}

fn openapi_info() -> OpenApiInfo {
    OpenApiInfo {
        title: "Authenticator API".to_owned(),
        version: "1.0.0".to_owned(),
        description: Some(
            "OIDC login, opaque sessions, and the cookie-to-JWT exchange behind \
             the nginx gateway (the BFF / token-handler pattern). The gateway \
             proxies /auth/* to this service and calls /internal/authz as its \
             auth_request target; /.well-known/* serves the discovery document \
             and JWKS downstream verifiers consume."
                .to_owned(),
        ),
        servers: Vec::new(),
    }
}

/// Build the authenticator `OpenAPI` document **offline** — no `AppState`,
/// Redis, IdP, or HTTP listener. Backs the `authenticator openapi` subcommand
/// (committed-doc regeneration + drift gate), reusing the exact
/// `build_operations` route table the live gear serves, so the two can never
/// diverge. The service-token listener (`POST /internal/token`, DD-AUTH-05) is
/// a separate raw-axum port outside `build_operations`, so it is deliberately
/// absent here.
pub fn openapi_document() -> anyhow::Result<utoipa::openapi::OpenApi> {
    let openapi = OpenApiRegistryImpl::new();
    let _ = build_operations(Router::new(), &openapi);
    openapi
        .build_openapi(&openapi_info())
        .map_err(|e| anyhow::anyhow!("failed to build authenticator OpenAPI document: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the full route table + `OpenAPI` registration with no
    /// `AppState`, Redis, or IdP: handlers are only *registered* (state comes
    /// in via `Extension` at serve time), never invoked. Guards against
    /// overlapping-route panics / bad `OperationBuilder` state.
    #[test]
    fn build_operations_registers_the_full_table_without_state() {
        let openapi = OpenApiRegistryImpl::new();
        let _router: Router = build_operations(Router::new(), &openapi);
    }
}
