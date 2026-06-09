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
use crate::bff::handlers::{BffState, callback, login, logout, me, refresh};

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

    let s = state.clone();
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

    let s = state.clone();
    router = OperationBuilder::new(Method::POST, "/auth/refresh")
        .summary("Rotate the session cookie and extend its TTL")
        .description(
            "Mints a fresh opaque SID, updates Redis indexes atomically, and \
             invalidates the Router's cached gateway JWT for the old SID. Returns \
             `{expires_at, refresh_at}` per DD-BFF-07; the SPA schedules its next \
             refresh from `refresh_at`. A grace window absorbs benign multi-tab \
             races (DD-BFF-10). 401 + clear cookie when the cookie is missing, \
             unknown, or past the absolute lifetime cap.",
        )
        .public()
        .json_response(StatusCode::OK, "Rotation succeeded")
        .json_response(StatusCode::UNAUTHORIZED, "No or invalid session")
        .handler(move |headers: HeaderMap| {
            let s = s.clone();
            async move { unify(refresh::refresh(State(s), headers).await) }
        })
        .register(router, openapi);

    let s = state;
    router = OperationBuilder::new(Method::POST, "/auth/logout")
        .summary("Revoke the current session and clear the cookie")
        .description(
            "Deletes the session record, removes it from the per-user index, \
             drops the IdP-sid index entry, and invalidates the Router's cached \
             gateway JWT. Returns `{end_session_url}` for SPA-driven RP-initiated \
             logout — `null` when the IdP did not advertise an end-session \
             endpoint. Idempotent: a request without a cookie still returns 200 \
             with a clear-cookie header.",
        )
        .public()
        .json_response(StatusCode::OK, "Logout complete")
        .handler(move |headers: HeaderMap| {
            let s = s.clone();
            async move { unify(logout::logout(State(s), headers).await) }
        })
        .register(router, openapi);

    tracing::info!(
        "BFF: registered /auth/login, /auth/callback, /auth/me, /auth/refresh, /auth/logout"
    );
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
