//! Route registration for `/auth/*` endpoints.
//!
//! All routes register as `.public()`. The BFF handles its own auth: every
//! protected endpoint reads the `__Host-sid` cookie, validates against
//! Redis, and returns 401 itself. We do NOT delegate cookie validation to
//! the existing `oidc-authn-plugin` (Bearer-JWT validator) — that plugin
//! is for upstream `/api/*` calls.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::IntoResponse;
use modkit::api::{OpenApiRegistry, OperationBuilder};

use crate::bff::errors::BffError;
use crate::bff::handlers::{BffState, callback, login, me};

pub fn register(mut router: Router, openapi: &dyn OpenApiRegistry, state: Arc<BffState>) -> Router {
    let s = state.clone();
    router = OperationBuilder::new(Method::GET, "/auth/login")
        .summary("Start OIDC login flow")
        .description(
            "Redirects the browser to the configured OIDC provider's authorize endpoint. \
             Public, no auth required. Pass `?return_to=/<path>` to land back on a \
             specific SPA page after callback.",
        )
        .public()
        .json_response(StatusCode::FOUND, "Redirect to IdP")
        .handler(move |q: Query<login::LoginQuery>| {
            let s = s.clone();
            async move { unify(login::login(State(s), q).await) }
        })
        .register(router, openapi);

    let s = state.clone();
    router = OperationBuilder::new(Method::GET, "/auth/callback")
        .summary("OIDC callback handler")
        .description(
            "Receives the authorization code from the IdP, exchanges it for an ID token, \
             validates the token, creates a session, sets the __Host-sid cookie, and \
             redirects to the SPA's target page.",
        )
        .public()
        .json_response(StatusCode::FOUND, "Redirect to SPA with session cookie")
        .handler(
            move |headers: HeaderMap, q: Query<callback::CallbackQuery>| {
                let s = s.clone();
                async move { unify(callback::callback(State(s), headers, q).await) }
            },
        )
        .register(router, openapi);

    let s = state;
    router = OperationBuilder::new(Method::GET, "/auth/me")
        .summary("Current session info")
        .description(
            "Returns the current user, tenant, expires_at, refresh_at, and csrf_token. \
             SPA calls this on boot to know whether the user is logged in and to schedule \
             the next /auth/refresh.",
        )
        .public()
        .json_response(StatusCode::OK, "Session view")
        .json_response(StatusCode::UNAUTHORIZED, "No or invalid session")
        .handler(move |headers: HeaderMap| {
            let s = s.clone();
            async move { unify(me::me(State(s), headers).await) }
        })
        .register(router, openapi);

    tracing::info!("BFF: registered /auth/login, /auth/callback, /auth/me");
    router
}

/// Collapse a handler `Result` into the unified `Response` shape that
/// axum / OperationBuilder expects.
fn unify(r: Result<axum::response::Response, BffError>) -> axum::response::Response {
    match r {
        Ok(resp) => resp,
        Err(e) => e.into_response(),
    }
}
