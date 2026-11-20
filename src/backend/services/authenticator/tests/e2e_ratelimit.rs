#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use serde::Deserialize;

const COOKIE: &str = "__Host-sid";

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn client() -> common::Client {
    common::client()
}

fn cookie_from(resp: &reqwest::Response) -> Option<String> {
    common::kc::session_cookie(resp)
}

async fn login(http: &common::Client, auth_base: &str, user: &str) -> String {
    common::kc::login(http, auth_base, user).await
}

async fn get_csrf(http: &common::Client, auth_base: &str, token: &str) -> String {
    #[derive(Deserialize)]
    struct CsrfBody {
        csrf_token: String,
    }
    let resp = http
        .get(format!("{auth_base}/auth/csrf"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json::<CsrfBody>().await.unwrap().csrf_token
}

async fn refresh(
    http: &common::Client,
    auth_base: &str,
    token: &str,
    csrf: &str,
) -> (u16, Option<String>) {
    let resp = http
        .post(format!("{auth_base}/auth/refresh"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .header("X-CSRF-Token", csrf)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let cookie = cookie_from(&resp);
    (status, cookie)
}

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn refresh_and_callback_buckets_trip_past_burst() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let http = client();

    let mut token = login(&http, &auth_base, &test_user).await;
    let csrf = get_csrf(&http, &auth_base, &token).await;
    let mut tripped_at = None;
    for attempt in 1..=8 {
        let (status, cookie) = refresh(&http, &auth_base, &token, &csrf).await;
        match status {
            200 => token = cookie.expect("200 refresh re-issues the cookie"),
            429 => {
                tripped_at = Some(attempt);
                break;
            }
            other => panic!("unexpected refresh status {other} on attempt {attempt}"),
        }
    }
    let tripped_at = tripped_at.expect("the refresh bucket must trip within 8 rapid attempts");
    assert!(
        tripped_at > 3,
        "the burst must absorb a few legitimate retries, tripped at {tripped_at}"
    );

    let other = login(&http, &auth_base, &test_user).await;
    let other_csrf = get_csrf(&http, &auth_base, &other).await;
    let (status, _) = refresh(&http, &auth_base, &other, &other_csrf).await;
    assert_eq!(status, 200, "another session must have its own bucket");

    let mut saw_429 = false;
    for _ in 1..=8 {
        let resp = http
            .get(format!(
                "{auth_base}/auth/callback?code=x&state=rl-e2e-bogus-state"
            ))
            .send()
            .await
            .unwrap();
        match resp.status().as_u16() {
            302 => {}
            429 => {
                saw_429 = true;
                break;
            }
            other => panic!("unexpected callback status {other}"),
        }
    }
    assert!(
        saw_429,
        "the per-state callback bucket must trip within 8 attempts"
    );
}
