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
    let victim = "refresh-victim@example.com";
    let survivor = "refresh-survivor@example.com";
    let http = client();

    let victim_token = common::kc::login(&http, &auth_base, victim).await;
    let survivor_token = common::kc::login(&http, &auth_base, survivor).await;
    assert_eq!(authz_status(&http, &auth_base, &victim_token).await, 200);
    assert_eq!(authz_status(&http, &auth_base, &survivor_token).await, 200);

    let outage = common::kc::idp_outage();
    tokio::time::sleep(Duration::from_secs(12)).await;
    assert_eq!(
        authz_status(&http, &auth_base, &victim_token).await,
        200,
        "an IdP outage must not log users out (fail open on transport)"
    );
    assert_eq!(authz_status(&http, &auth_base, &survivor_token).await, 200);
    drop(outage);

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

    assert_eq!(
        authz_status(&http, &auth_base, &survivor_token).await,
        200,
        "an unrelated user must survive another user's invalid_grant kill"
    );
}
