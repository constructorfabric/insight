//! End-to-end session management against a running authenticator + fakeidp +
//! Redis (nginx+auth step 10, item 2).
//!
//! `#[ignore]` by default (needs the stack up; `run-e2e.sh` drives it):
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 \
//!   cargo test -p authenticator --test e2e_sessions -- --ignored --nocapture
//! ```
//!
//! Covers: listing active sessions (current flag, attribution fields), revoking
//! a specific other session, the no-existence-oracle 404, and "log out
//! everywhere". The admin revoke-by-user variant needs the gateway-JWT authn
//! pipeline (TLS discovery front) and is exercised in the compose e2e instead.

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

/// Fetch the session's CSRF token (state-changing /auth/* requires it, 10.5).
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
    assert_eq!(resp.status(), 200, "GET /auth/csrf must succeed");
    resp.json::<CsrfBody>().await.unwrap().csrf_token
}

/// Run the full fakeidp login loop; returns the session cookie token.
async fn login(http: &reqwest::Client, auth_base: &str, user: &str) -> String {
    let login = http
        .get(format!("{auth_base}/auth/login"))
        .header(reqwest::header::USER_AGENT, "e2e-sessions-test")
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
    let cb = http
        .get(&callback)
        .header(reqwest::header::USER_AGENT, "e2e-sessions-test")
        .send()
        .await
        .unwrap();
    assert_eq!(cb.status(), 302);
    cookie_from(&cb).expect("callback must set __Host-sid")
}

#[derive(Deserialize)]
struct SessionItem {
    session_id: String,
    created_at: u64,
    expires_at: u64,
    user_agent: String,
    #[allow(dead_code)]
    ip: String,
    current: bool,
}

#[derive(Deserialize)]
struct SessionsBody {
    sessions: Vec<SessionItem>,
}

async fn list(http: &reqwest::Client, auth_base: &str, token: &str) -> Vec<SessionItem> {
    let resp = http
        .get(format!("{auth_base}/auth/sessions"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json::<SessionsBody>().await.unwrap().sessions
}

#[tokio::test]
#[ignore = "requires a running authenticator + fakeidp + Redis stack"]
async fn sessions_list_revoke_and_logout_everywhere() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let http = client();

    // Two devices: two independent logins for the same person.
    let token_a = login(&http, &auth_base, &test_user).await;
    let token_b = login(&http, &auth_base, &test_user).await;
    let csrf_b = get_csrf(&http, &auth_base, &token_b).await;

    // 1. The list shows both sessions, flags the caller's as current, and
    //    carries the attribution captured at login.
    // Device A's own id, resolved from its own list (the person may carry
    // leftover sessions from earlier e2e tests — same deterministic stub user).
    let session_a_id = list(&http, &auth_base, &token_a)
        .await
        .into_iter()
        .find(|s| s.current)
        .expect("device A must see itself as current")
        .session_id;

    let sessions = list(&http, &auth_base, &token_b).await;
    assert!(
        sessions.len() >= 2,
        "both live sessions must be listed, got {}",
        sessions.len()
    );
    let current = sessions
        .iter()
        .find(|s| s.current)
        .expect("the caller's session must be flagged current");
    assert!(current.expires_at > current.created_at);
    assert_eq!(
        current.user_agent, "e2e-sessions-test",
        "user_agent captured at login must be surfaced"
    );
    let other = sessions
        .iter()
        .find(|s| s.session_id == session_a_id)
        .expect("the other device's session must be listed");
    assert!(!other.current, "device A is not current for device B");

    // 2. Revoking an unknown/foreign session id → 404 (no existence oracle).
    let bogus = http
        .delete(format!(
            "{auth_base}/auth/sessions/00000000-0000-7000-8000-000000000000"
        ))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_b}"))
        .header("X-CSRF-Token", &csrf_b)
        .send()
        .await
        .unwrap();
    assert_eq!(bogus.status(), 404);

    // 3. Revoke the other device's session; it dies, the caller's survives.
    let revoke = http
        .delete(format!("{auth_base}/auth/sessions/{}", other.session_id))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_b}"))
        .header("X-CSRF-Token", &csrf_b)
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status(), 200);
    let after = http
        .get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_a}"))
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), 401, "revoked device must be logged out");
    let survivors = list(&http, &auth_base, &token_b).await;
    assert!(survivors.iter().all(|s| s.session_id != other.session_id));

    // 4. Log out everywhere: every session dies, cookie cleared.
    // Without the CSRF token the destructive call is refused (10.5)…
    let no_csrf = http
        .delete(format!("{auth_base}/auth/sessions"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_b}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        no_csrf.status(),
        403,
        "log-out-everywhere without CSRF must be 403"
    );

    // …and with it, every session dies.
    let all = http
        .delete(format!("{auth_base}/auth/sessions"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_b}"))
        .header("X-CSRF-Token", &csrf_b)
        .send()
        .await
        .unwrap();
    assert_eq!(all.status(), 200);
    let cleared = all
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .any(|h| h.to_str().unwrap_or("").contains("Max-Age=0"));
    assert!(cleared, "log-out-everywhere must clear the cookie");
    let dead = http
        .get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_b}"))
        .send()
        .await
        .unwrap();
    assert_eq!(dead.status(), 401);
}
