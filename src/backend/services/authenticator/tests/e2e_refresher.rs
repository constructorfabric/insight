//! End-to-end IdP background refresher (nginx+auth step 10, item 4) against a
//! running authenticator + fakeidp + Redis with a FAST refresh lifecycle
//! (`run-e2e.sh` sets `FAKEIDP_TOKEN_TTL=15`, margin 10 s, tick 1 s, jitter
//! ±1 s, so a session's IdP tokens refresh every ~5 s).
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 FAKEIDP_PUBLIC=http://localhost:8084 \
//!   cargo test -p authenticator --test e2e_refresher -- --ignored --nocapture
//! ```
//!
//! Drives fakeidp's control hooks (the reason fakeidp exists, G6):
//! `/_control/outage` — transient failures must log nobody out; and
//! `/_control/revoke/{user}` — the definitive `invalid_grant` verdict must
//! kill the user's sessions on the next scheduled refresh, while another
//! user's session survives.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

use std::time::Duration;

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

async fn authz_status(http: &reqwest::Client, auth_base: &str, token: &str) -> u16 {
    http.get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn control(http: &reqwest::Client, idp: &str, path: &str, body: Option<serde_json::Value>) {
    let req = http.post(format!("{idp}{path}"));
    let req = match body {
        Some(json) => req.json(&json),
        None => req,
    };
    let resp = req.send().await.unwrap();
    assert!(
        resp.status().is_success(),
        "control hook {path} failed: {}",
        resp.status()
    );
}

/// Poll `authz` until it returns `expected` or the deadline passes.
async fn wait_for_status(
    http: &reqwest::Client,
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
    let idp = env("FAKEIDP_PUBLIC", "http://localhost:8084");
    let victim = "alice@example.com";
    let survivor = "bob@example.com";
    let http = client();

    let victim_token = login(&http, &auth_base, victim).await;
    let survivor_token = login(&http, &auth_base, survivor).await;
    assert_eq!(authz_status(&http, &auth_base, &victim_token).await, 200);
    assert_eq!(authz_status(&http, &auth_base, &survivor_token).await, 200);

    // 1. Outage: the IdP token endpoint returns 5xx. Refresh attempts fail
    //    TRANSIENTLY for ~12 s (several due cycles at the fast lifecycle) —
    //    nobody may be logged out by a blip.
    control(
        &http,
        &idp,
        "/_control/outage",
        Some(serde_json::json!({"mode": "5xx"})),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(12)).await;
    assert_eq!(
        authz_status(&http, &auth_base, &victim_token).await,
        200,
        "an IdP outage must not log users out (fail open on transport)"
    );
    assert_eq!(authz_status(&http, &auth_base, &survivor_token).await, 200);
    control(
        &http,
        &idp,
        "/_control/outage",
        Some(serde_json::json!({"mode": "off"})),
    )
    .await;

    // 2. Definitive verdict: revoke the victim at the IdP. The next scheduled
    //    refresh gets invalid_grant and the session dies through the standard
    //    pipeline. Generous deadline: the outage above pushed the session into
    //    exponential backoff (~15–40 s).
    control(&http, &idp, &format!("/_control/revoke/{victim}"), None).await;
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
