//! End-to-end `__override` (view-as, #1941) against a running authenticator +
//! fakeidp + Redis + identity stub.
//!
//! `#[ignore]` by default (needs the stack up; `run-e2e.sh` drives it):
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 AUTH_BASE_DISABLED=http://localhost:8085 \
//!   cargo test -p authenticator --test e2e_override -- --ignored --nocapture
//! ```
//!
//! Covers: a login carrying `__override=<email>` mints the session for that
//! person (JWT `sub`, `/auth/me` user/email) with the real principal recorded
//! (`impersonator_email`); an unknown target is 403; and against an instance
//! with `override_enabled` at its default (`false`) the parameter is inert.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

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

/// Run the fakeidp login loop as `user`, optionally with `__override=<email>`
/// on `/auth/login`. Returns the callback response (302 + cookie on success,
/// the error status otherwise).
async fn login_flow(
    http: &reqwest::Client,
    auth_base: &str,
    user: &str,
    override_email: Option<&str>,
) -> reqwest::Response {
    let mut login_url = format!("{auth_base}/auth/login");
    if let Some(email) = override_email {
        login_url = format!("{login_url}?__override={}", urlencode(email));
    }
    let login = http.get(&login_url).send().await.unwrap();
    assert_eq!(login.status(), 302, "GET /auth/login must redirect");
    let authorize = login.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned();
    let sep = if authorize.contains('?') { '&' } else { '?' };
    let authorized = http
        .get(format!("{authorize}{sep}user={user}"))
        .send()
        .await
        .unwrap();
    assert_eq!(authorized.status(), 302, "fakeidp /authorize must redirect");
    let callback = authorized.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned();
    http.get(&callback).send().await.unwrap()
}

fn urlencode(s: &str) -> String {
    s.replace('@', "%40").replace('+', "%2B")
}

async fn me(http: &reqwest::Client, auth_base: &str, token: &str) -> serde_json::Value {
    let resp = http
        .get(format!("{auth_base}/auth/me"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "GET /auth/me must succeed");
    resp.json().await.unwrap()
}

/// The JWT `sub` behind a session cookie, via the `/internal/authz` exchange.
async fn jwt_sub(http: &reqwest::Client, auth_base: &str, token: &str) -> String {
    let resp = http
        .get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "/internal/authz must exchange");
    let bearer = resp.headers()["x-gateway-jwt"].to_str().unwrap().to_owned();
    let jwt = bearer.strip_prefix("Bearer ").unwrap();
    let payload = B64.decode(jwt.split('.').nth(1).unwrap()).unwrap();
    let claims: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    claims["sub"].as_str().unwrap().to_owned()
}

#[tokio::test]
#[ignore = "needs the e2e stack (run-e2e.sh)"]
async fn override_mints_the_session_for_the_target_person() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let user = env("E2E_USER", "dev@company.nonpresent");
    let target = "bob@example.com";
    let http = client();

    // Baseline: a normal login AS the target — its person id is the reference
    // the override must reproduce (the identity stub is deterministic).
    let cb = login_flow(&http, &auth_base, target, None).await;
    assert_eq!(cb.status(), 302);
    let token = cookie_from(&cb).expect("baseline callback must set the cookie");
    let baseline = me(&http, &auth_base, &token).await;
    let target_person_id = baseline["user"].as_str().unwrap().to_owned();
    assert!(
        baseline.get("impersonator_email").is_none(),
        "a normal login must not carry impersonator_email"
    );

    // The override login: authenticate as `user`, view as `target`.
    let cb = login_flow(&http, &auth_base, &user, Some(target)).await;
    assert_eq!(cb.status(), 302, "override callback must succeed");
    let token = cookie_from(&cb).expect("override callback must set the cookie");

    let body = me(&http, &auth_base, &token).await;
    assert_eq!(body["email"].as_str().unwrap(), target);
    assert_eq!(body["user"].as_str().unwrap(), target_person_id);
    assert_eq!(
        body["impersonator_email"].as_str().unwrap(),
        user,
        "/auth/me must name the real principal behind the view-as session"
    );

    // The minted JWT acts as the target everywhere downstream.
    assert_eq!(jwt_sub(&http, &auth_base, &token).await, target_person_id);

    // The view-as session is reachable through the REAL principal: it is
    // indexed under both persons, so the impersonator's own "log out
    // everywhere" must kill it.
    let cb = login_flow(&http, &auth_base, &user, None).await;
    assert_eq!(cb.status(), 302);
    let own_token = cookie_from(&cb).expect("normal login must set the cookie");
    let csrf = csrf_token(&http, &auth_base, &own_token).await;
    let all = http
        .delete(format!("{auth_base}/auth/sessions"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={own_token}"))
        .header("X-CSRF-Token", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(all.status(), 200, "revoke-all as the impersonator");
    let dead = http
        .get(format!("{auth_base}/auth/me"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        dead.status(),
        401,
        "the impersonator's revoke-all must reach their view-as session"
    );
}

/// The session-bound CSRF token (state-changing `/auth/*` requires it).
async fn csrf_token(http: &reqwest::Client, auth_base: &str, token: &str) -> String {
    let resp = http
        .get(format!("{auth_base}/auth/csrf"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "GET /auth/csrf must succeed");
    resp.json::<serde_json::Value>().await.unwrap()["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
#[ignore = "needs the e2e stack (run-e2e.sh)"]
async fn override_with_unknown_target_is_denied() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let user = env("E2E_USER", "dev@company.nonpresent");
    let http = client();

    // The identity stub 404s emails prefixed `unknown-` (test seam).
    let cb = login_flow(&http, &auth_base, &user, Some("unknown-nobody@example.com")).await;
    assert_eq!(
        cb.status(),
        403,
        "an unknown override target must be denied"
    );
    assert!(cookie_from(&cb).is_none(), "no session may be minted");
}

#[tokio::test]
#[ignore = "needs the e2e stack (run-e2e.sh)"]
async fn override_is_inert_when_disabled() {
    // The second instance runs with `override_enabled` at its default (false).
    let auth_base = env("AUTH_BASE_DISABLED", "http://localhost:8085");
    let user = env("E2E_USER", "dev@company.nonpresent");
    let http = client();

    let cb = login_flow(&http, &auth_base, &user, Some("bob@example.com")).await;
    assert_eq!(cb.status(), 302, "login itself must still succeed");
    let token = cookie_from(&cb).expect("callback must set the cookie");

    let body = me(&http, &auth_base, &token).await;
    assert_eq!(
        body["email"].as_str().unwrap(),
        user,
        "with the flag off the session must be the caller's own"
    );
    assert!(
        body.get("impersonator_email").is_none(),
        "no impersonation marker on an inert override"
    );
}
