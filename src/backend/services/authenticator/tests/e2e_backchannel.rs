//! End-to-end OIDC back-channel logout against a running authenticator +
//! Keycloak + Redis (nginx+auth step 10, item 3). The realm registers the
//! authenticator's `/auth/oidc/back-channel-logout` as the client's
//! back-channel logout URL (`run-e2e.sh` wires it via kc-realm-overlay.py):
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 \
//!   cargo test -p authenticator --test e2e_backchannel -- --ignored --nocapture
//! ```
//!
//! An admin-triggered IdP logout makes Keycloak fire a signed `logout_token`
//! per session at the authenticator, and every session of that user dies
//! through the standard revoke pipeline.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

mod common;

const COOKIE: &str = "__Host-sid";
// Dedicated realm user (kc-realm-overlay.py): the IdP-side logout must never
// reach another suite's live sessions.
const USER: &str = "backchannel@example.com";

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn client() -> common::Client {
    common::client()
}

async fn authz_status(http: &common::Client, auth_base: &str, token: &str) -> u16 {
    http.get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn back_channel_logout_kills_the_users_sessions() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let http = client();

    // Two devices, both alive.
    let token_a = common::kc::login(&http, &auth_base, USER).await;
    let token_b = common::kc::login(&http, &auth_base, USER).await;
    assert_eq!(authz_status(&http, &auth_base, &token_a).await, 200);
    assert_eq!(authz_status(&http, &auth_base, &token_b).await, 200);

    // The IdP-side logout: Keycloak fires a signed logout_token per session
    // at the registered back-channel URL. Each device logged in separately,
    // so each has its own OIDC `sid` — both sid-index revokes must land.
    common::kc::logout_user(USER).await;

    // Delivery is the IdP's side of the contract and asynchronous; poll
    // briefly rather than assert the first read.
    for (device, token) in [("A", &token_a), ("B", &token_b)] {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if authz_status(&http, &auth_base, token).await == 401 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "device {device} must be logged out by back-channel logout"
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    // A rejected (unsigned garbage) token is a 400, not a revoke.
    let bad = http
        .post(format!("{auth_base}/auth/oidc/back-channel-logout"))
        .form(&[("logout_token", "garbage.token.value")])
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400, "a malformed logout_token must be 400");
}
