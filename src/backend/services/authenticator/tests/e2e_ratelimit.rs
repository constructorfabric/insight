//! End-to-end layer-2 rate limiting (nginx+auth step 10, item 6) against a
//! running authenticator + fakeidp + Redis.
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 \
//!   cargo test -p authenticator --test e2e_ratelimit -- --ignored --nocapture
//! ```
//!
//! Asserts the Redis token buckets (defaults: refresh 5-burst/6-per-min per
//! session, callback 5-burst/10-per-min per state) answer 429 past the burst,
//! and that the limit keys on the session — a second session is unaffected.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde::Deserialize;

const COOKIE: &str = "__Host-sid";

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn rewrite_host(url: &str) -> String {
    match (
        std::env::var("FAKEIDP_REWRITE_FROM"),
        std::env::var("FAKEIDP_REWRITE_TO"),
    ) {
        (Ok(from), Ok(to)) if !from.is_empty() => url.replace(&from, &to),
        _ => url.to_owned(),
    }
}

fn cookie_from(resp: &reqwest::Response) -> Option<String> {
    for hv in resp.headers().get_all(reqwest::header::SET_COOKIE) {
        let raw = hv.to_str().ok()?;
        for part in raw.split(';') {
            if let Some(v) = part.trim().strip_prefix(&format!("{COOKIE}="))
                && !v.is_empty()
            {
                return Some(v.to_owned());
            }
        }
    }
    None
}

async fn login(http: &reqwest::Client, auth_base: &str, user: &str) -> String {
    let login = http
        .get(format!("{auth_base}/auth/login"))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 302);
    let authorize = rewrite_host(login.headers()[reqwest::header::LOCATION].to_str().unwrap());
    let sep = if authorize.contains('?') { '&' } else { '?' };
    let authorized = http
        .get(format!("{authorize}{sep}user={user}"))
        .send()
        .await
        .unwrap();
    assert_eq!(authorized.status(), 302);
    let callback = rewrite_host(
        authorized.headers()[reqwest::header::LOCATION]
            .to_str()
            .unwrap(),
    );
    let cb = http.get(&callback).send().await.unwrap();
    assert_eq!(cb.status(), 302);
    cookie_from(&cb).expect("callback must set __Host-sid")
}

async fn get_csrf(http: &reqwest::Client, auth_base: &str, token: &str) -> String {
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

/// One refresh attempt; returns (status, rotated cookie when present).
async fn refresh(
    http: &reqwest::Client,
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
#[ignore = "requires a running authenticator + fakeidp + Redis stack"]
async fn refresh_and_callback_buckets_trip_past_burst() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let http = client();

    // 1. Refresh bucket: hammer one session until the (default 5-burst)
    //    bucket trips. The credential rotates on each 200; the bucket keys on
    //    the STABLE session id, so rotation does not reset it.
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

    // 2. A different session is unaffected (the bucket keys per session).
    let other = login(&http, &auth_base, &test_user).await;
    let other_csrf = get_csrf(&http, &auth_base, &other).await;
    let (status, _) = refresh(&http, &auth_base, &other, &other_csrf).await;
    assert_eq!(status, 200, "another session must have its own bucket");

    // 3. Callback bucket: hammering one (bogus) state flips from 400
    //    (unknown state) to 429 once the per-state bucket empties.
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
            400 => {}
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
