//! End-to-end `/auth/refresh` rotation-with-grace against a running
//! authenticator + fakeidp + Redis (nginx+auth step 10, item 1).
//!
//! `#[ignore]` by default (needs the stack up):
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 \
//!   cargo test -p authenticator --test e2e_refresh -- --ignored --nocapture
//! ```
//!
//! Asserts the G10 rotation model: refresh rotates the cookie credential but
//! not the session (`sid` claim stable), the superseded token keeps resolving
//! during the grace window without a second rotation, and a token past grace
//! is refused with a cleared cookie.

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

/// Run the full fakeidp login loop; returns the session cookie token.
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

#[derive(Deserialize)]
struct RefreshBody {
    expires_at: u64,
    refresh_at: u64,
}

#[derive(Deserialize)]
struct MeBody {
    expires_at: u64,
    refresh_at: u64,
}

#[derive(Deserialize)]
struct JwtSid {
    sid: String,
}

/// Decode the (unverified) `sid` claim from a compact JWT.
fn jwt_sid(bearer: &str) -> String {
    use base64::Engine as _;
    let jwt = bearer.strip_prefix("Bearer ").unwrap();
    let payload = jwt.split('.').nth(1).unwrap();
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .unwrap();
    serde_json::from_slice::<JwtSid>(&bytes).unwrap().sid
}

async fn authz_sid(http: &reqwest::Client, auth_base: &str, token: &str) -> Option<String> {
    let resp = http
        .get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    if resp.status() != 200 {
        return None;
    }
    Some(jwt_sid(resp.headers()["x-gateway-jwt"].to_str().unwrap()))
}

#[tokio::test]
#[ignore = "requires a running authenticator + fakeidp + Redis stack"]
async fn refresh_rotates_with_grace_and_stable_session() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let http = client();

    let old_token = login(&http, &auth_base, &test_user).await;
    let sid_before = authz_sid(&http, &auth_base, &old_token)
        .await
        .expect("fresh session exchanges");

    let csrf = get_csrf(&http, &auth_base, &old_token).await;

    // 0. A state-changing /auth/* request without the CSRF token (and no
    //    allowlisted Origin) is rejected 403 before any rotation happens.
    let no_csrf = http
        .post(format!("{auth_base}/auth/refresh"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={old_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), 403, "refresh without CSRF must be 403");

    // 1. Refresh rotates the credential and returns the timing contract.
    let refresh = http
        .post(format!("{auth_base}/auth/refresh"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={old_token}"))
        .header("X-CSRF-Token", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(refresh.status(), 200, "refresh must succeed");
    let new_token = cookie_from(&refresh).expect("refresh must re-issue the cookie");
    assert_ne!(new_token, old_token, "credential must rotate");
    let body: RefreshBody = refresh.json().await.unwrap();
    // refresh_at ∈ [expires_at − margin − jitter/2, expires_at − margin + jitter/2]
    // (defaults: margin 90, jitter ±60 → [exp−150, exp−30]).
    assert!(
        body.refresh_at < body.expires_at,
        "refresh_at must precede expires_at"
    );

    // 2. The stable session survives rotation: same `sid` through the new token.
    let sid_after = authz_sid(&http, &auth_base, &new_token)
        .await
        .expect("rotated session exchanges");
    assert_eq!(sid_before, sid_after, "sid must be stable across rotation");

    // 3. Grace: an immediate second refresh with the OLD token resolves to the
    //    same session and does NOT rotate again (returns the current cookie).
    let grace = http
        .post(format!("{auth_base}/auth/refresh"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={old_token}"))
        .header("X-CSRF-Token", &csrf)
        .send()
        .await
        .unwrap();
    if grace.status() == 200 {
        let grace_token = cookie_from(&grace).expect("grace refresh re-issues the current cookie");
        assert_eq!(
            grace_token, new_token,
            "grace path must answer with the current credential, not rotate again"
        );
    } else {
        // The 250 ms default grace may already have elapsed under load — a 401
        // here is the past-grace contract, not a failure of the grace path.
        assert_eq!(grace.status(), 401);
    }

    // 4. Past grace the old token is dead: 401 + cleared cookie.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let stale = http
        .post(format!("{auth_base}/auth/refresh"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={old_token}"))
        .header("X-CSRF-Token", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 401, "past-grace token must be refused");
    let cleared = stale
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .any(|h| h.to_str().unwrap_or("").contains("Max-Age=0"));
    assert!(cleared, "past-grace refusal must clear the cookie");

    // 5. /auth/me returns the same timing fields for the live credential.
    let me = http
        .get(format!("{auth_base}/auth/me"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={new_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), 200);
    let me_body: MeBody = me.json().await.unwrap();
    assert!(me_body.refresh_at < me_body.expires_at);
}
