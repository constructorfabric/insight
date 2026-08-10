//! End-to-end 401 contract for the session-cookie surface: every `.public()`
//! route that requires a session answers 401 both without a cookie and with a
//! cookie that resolves to no session.
//!
//! `#[ignore]` by default (needs the stack up; `run-e2e.sh` drives it):
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 \
//!   cargo test -p authenticator --test e2e_unauthorized -- --ignored --nocapture
//! ```
//!
//! Covers the 401 responses declared in the OpenAPI spec for /auth/me,
//! /auth/csrf, and the /auth/sessions family; /auth/refresh and
//! /internal/authz 401s are exercised by `e2e_refresh` and `e2e_login_loop`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use reqwest::Method;

const COOKIE: &str = "__Host-sid";

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

const SESSION_ROUTES: &[(Method, &str)] = &[
    (Method::GET, "/auth/me"),
    (Method::GET, "/auth/csrf"),
    (Method::GET, "/auth/sessions"),
    (Method::DELETE, "/auth/sessions"),
    (
        Method::DELETE,
        "/auth/sessions/00000000-0000-7000-8000-000000000000",
    ),
];

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn session_routes_reject_missing_cookie_with_401() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let http = common::client();

    for (method, path) in SESSION_ROUTES {
        let resp = http
            .request(method.clone(), format!("{auth_base}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            401,
            "{method} {path} without a cookie must be 401"
        );
        let body = resp.text().await.unwrap();
        assert_eq!(
            body, r#"{"error":"unauthenticated"}"#,
            "{method} {path} must return the unauthenticated body"
        );
    }
}

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn session_routes_reject_unknown_session_token_with_401() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let http = common::client();

    for (method, path) in SESSION_ROUTES {
        let resp = http
            .request(method.clone(), format!("{auth_base}{path}"))
            .header(
                reqwest::header::COOKIE.as_str(),
                format!("{COOKIE}=e2e-unauthorized-not-a-session"),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            401,
            "{method} {path} with an unknown session token must be 401"
        );
    }
}
