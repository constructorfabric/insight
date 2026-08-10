//! End-to-end IdP background refresher (nginx+auth step 10, item 4) against a
//! running authenticator + Keycloak + Redis with a FAST refresh lifecycle
//! (`run-e2e.sh` pairs the realm's 15 s access-token lifespan with margin
//! 10 s, tick 1 s, jitter ±1 s, so a session's IdP tokens refresh every ~5 s).
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 \
//!   cargo test -p authenticator --test e2e_refresher -- --ignored --nocapture
//! ```
//!
//! Drives the two IdP failure modes through real-IdP seams
//! (tests/common/kc.rs): a paused container —
//! transient failures must log nobody out; and an admin-disabled user — the
//! definitive `invalid_grant` verdict must kill the user's sessions on the
//! next scheduled refresh, while another user's session survives.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

mod common;

use std::time::Duration;

const COOKIE: &str = "__Host-sid";

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

/// Poll `authz` until it returns `expected` or the deadline passes.
async fn wait_for_status(
    http: &common::Client,
    auth_base: &str,
    token: &str,
    expected: u16,
    deadline: Duration,
) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if authz_status(http, auth_base, token).await == expected {
            return true;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    false
}

#[tokio::test]
#[ignore = "requires the fast-lifecycle e2e stack (run-e2e.sh)"]
async fn refresher_outage_survives_and_invalid_grant_kills() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    // Dedicated realm users (kc-realm-overlay.py): the victim stays disabled
    // at the IdP after this test, so no other suite may share it.
    let victim = "refresh-victim@example.com";
    let survivor = "refresh-survivor@example.com";
    let http = client();

    let victim_token = common::kc::login(&http, &auth_base, victim).await;
    let survivor_token = common::kc::login(&http, &auth_base, survivor).await;
    assert_eq!(authz_status(&http, &auth_base, &victim_token).await, 200);
    assert_eq!(authz_status(&http, &auth_base, &survivor_token).await, 200);

    // 1. Outage: the IdP container is paused, so refresh attempts hang until
    //    the OIDC client's own timeout and fail TRANSIENTLY for ~12 s (several
    //    due cycles at the fast lifecycle) — nobody may be logged out by a blip.
    //    The guard unpauses on drop even if an assertion fails mid-outage.
    let outage = common::kc::idp_outage();
    tokio::time::sleep(Duration::from_secs(12)).await;
    assert_eq!(
        authz_status(&http, &auth_base, &victim_token).await,
        200,
        "an IdP outage must not log users out (fail open on transport)"
    );
    assert_eq!(authz_status(&http, &auth_base, &survivor_token).await, 200);
    drop(outage);

    // 2. Definitive verdict: disable the victim at the IdP. The next scheduled
    //    refresh gets invalid_grant and the session dies through the standard
    //    pipeline. (An admin logout would ALSO fire back-channel logout — the
    //    disable keeps the kill on the refresher path this test pins.)
    //    Generous deadline: the outage above pushed the session into
    //    exponential backoff (~15–40 s).
    common::kc::set_user_enabled(victim, false).await;
    let died = wait_for_status(
        &http,
        &auth_base,
        &victim_token,
        401,
        Duration::from_secs(90),
    )
    .await;
    assert!(
        died,
        "the revoked user's session must die on the next scheduled refresh"
    );

    // 3. The other user's session lives on — the kill is per grant, not global.
    assert_eq!(
        authz_status(&http, &auth_base, &survivor_token).await,
        200,
        "an unrelated user must survive another user's invalid_grant kill"
    );
}
