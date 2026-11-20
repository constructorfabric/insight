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
struct MePerson {
    user: String,
}

#[derive(Deserialize)]
struct JwtSid {
    sid: String,
}

fn jwt_sid(bearer: &str) -> String {
    use base64::Engine as _;
    let jwt = bearer.strip_prefix("Bearer ").unwrap();
    let payload = jwt.split('.').nth(1).unwrap();
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .unwrap();
    serde_json::from_slice::<JwtSid>(&bytes).unwrap().sid
}

async fn authz_sid(http: &common::Client, auth_base: &str, token: &str) -> Option<String> {
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
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn refresh_rotates_with_grace_and_stable_session() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let http = client();

    let old_token = common::kc::login(&http, &auth_base, &test_user).await;
    let sid_before = authz_sid(&http, &auth_base, &old_token)
        .await
        .expect("fresh session exchanges");

    let csrf = get_csrf(&http, &auth_base, &old_token).await;

    let no_csrf = http
        .post(format!("{auth_base}/auth/refresh"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={old_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), 403, "refresh without CSRF must be 403");

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
    assert!(
        body.refresh_at < body.expires_at,
        "refresh_at must precede expires_at"
    );

    let sid_after = authz_sid(&http, &auth_base, &new_token)
        .await
        .expect("rotated session exchanges");
    assert_eq!(sid_before, sid_after, "sid must be stable across rotation");

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
        assert_eq!(grace.status(), 401);
    }

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

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn refresh_answers_within_the_client_budget_when_identity_hangs() {
    const SPA_ABORT_SECS: u64 = 10;

    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let identity_base = env("IDENTITY_BASE", "http://localhost:8092");
    let test_user = "roles-stall@example.com";
    let http = client();

    let token = common::kc::login(&http, &auth_base, test_user).await;
    let csrf = get_csrf(&http, &auth_base, &token).await;

    let me = http
        .get(format!("{auth_base}/auth/me"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), 200, "the fresh session must resolve");
    let person_id = me.json::<MePerson>().await.unwrap().user;

    let armed = http
        .post(format!(
            "{identity_base}/__test/roles-stall?person_id={person_id}&seconds=30"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(armed.status(), 204, "the identity stub must arm the stall");

    let started = std::time::Instant::now();
    let refresh = http
        .post(format!("{auth_base}/auth/refresh"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .header("X-CSRF-Token", &csrf)
        .send()
        .await
        .unwrap();
    let took = started.elapsed();

    assert_eq!(
        refresh.status(),
        200,
        "a roles blip must not fail a refresh"
    );
    assert!(
        took < std::time::Duration::from_secs(SPA_ABORT_SECS),
        "refresh took {took:?}, past the {SPA_ABORT_SECS} s the client waits"
    );

    let rotated = cookie_from(&refresh).expect("refresh must re-issue the cookie");
    assert_ne!(rotated, token, "credential must rotate");
    assert!(
        authz_sid(&http, &auth_base, &rotated).await.is_some(),
        "the rotated credential must resolve"
    );
}
