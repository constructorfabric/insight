#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

const COOKIE: &str = "__Host-sid";

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn client() -> common::Client {
    common::client()
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

async fn login_flow(
    http: &common::Client,
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
    let callback = common::kc::authorize(&authorize, user).await;
    http.get(&callback).send().await.unwrap()
}

fn urlencode(s: &str) -> String {
    s.replace('@', "%40").replace('+', "%2B")
}

async fn me(http: &common::Client, auth_base: &str, token: &str) -> serde_json::Value {
    let resp = http
        .get(format!("{auth_base}/auth/me"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "GET /auth/me must succeed");
    resp.json().await.unwrap()
}

async fn jwt_sub(http: &common::Client, auth_base: &str, token: &str) -> String {
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
    let user = "viewer-mints@example.com";
    let target = "target-mints@example.com";
    let http = client();

    let cb = login_flow(&http, &auth_base, target, None).await;
    assert_eq!(cb.status(), 302);
    let token = cookie_from(&cb).expect("baseline callback must set the cookie");
    let baseline = me(&http, &auth_base, &token).await;
    let target_person_id = baseline["user"].as_str().unwrap().to_owned();
    assert!(
        baseline.get("impersonator_email").is_none(),
        "a normal login must not carry impersonator_email"
    );

    let cb = login_flow(&http, &auth_base, user, Some(target)).await;
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

    assert_eq!(jwt_sub(&http, &auth_base, &token).await, target_person_id);

    let cb = login_flow(&http, &auth_base, user, None).await;
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

async fn csrf_token(http: &common::Client, auth_base: &str, token: &str) -> String {
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
async fn override_relogin_swaps_the_target_and_kills_the_old_session() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let user = "viewer-relogin@example.com";
    let http = client();

    let cb = login_flow(
        &http,
        &auth_base,
        user,
        Some("target-relogin-a@example.com"),
    )
    .await;
    assert_eq!(cb.status(), 302);
    let token_a = cookie_from(&cb).expect("first override callback must set the cookie");
    let me_a = me(&http, &auth_base, &token_a).await;
    assert_eq!(
        me_a["email"].as_str().unwrap(),
        "target-relogin-a@example.com"
    );
    let sub_a = jwt_sub(&http, &auth_base, &token_a).await;

    let csrf = csrf_token(&http, &auth_base, &token_a).await;
    let out = http
        .post(format!("{auth_base}/auth/logout"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_a}"))
        .header("X-CSRF-Token", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200, "logout must succeed");

    let stale_me = http
        .get(format!("{auth_base}/auth/me"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_a}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        stale_me.status(),
        401,
        "old cookie must be dead after logout"
    );
    let stale_authz = http
        .get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_a}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        stale_authz.status(),
        401,
        "the exchange must refuse the revoked credential (nothing for nginx to re-cache)"
    );

    let cb = login_flow(
        &http,
        &auth_base,
        user,
        Some("target-relogin-b@example.com"),
    )
    .await;
    assert_eq!(cb.status(), 302);
    let token_b = cookie_from(&cb).expect("second override callback must set the cookie");
    assert_ne!(
        token_a, token_b,
        "the cookie credential must rotate across logins"
    );

    let me_b = me(&http, &auth_base, &token_b).await;
    assert_eq!(
        me_b["email"].as_str().unwrap(),
        "target-relogin-b@example.com"
    );
    assert_eq!(me_b["impersonator_email"].as_str().unwrap(), user);
    let sub_b = jwt_sub(&http, &auth_base, &token_b).await;
    assert_ne!(
        sub_a, sub_b,
        "the new JWT must carry the NEW target, not a cached one"
    );
    assert_eq!(
        me_b["user"].as_str().unwrap(),
        sub_b,
        "/auth/me and the exchanged JWT must agree on the identity"
    );
}

#[tokio::test]
#[ignore = "needs the e2e stack (run-e2e.sh)"]
async fn override_switch_without_logout_revokes_the_presented_session() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let user = "viewer-switch@example.com";
    let http = client();

    let cb = login_flow(&http, &auth_base, user, Some("target-switch-a@example.com")).await;
    assert_eq!(cb.status(), 302);
    let token_b = cookie_from(&cb).expect("first override callback must set the cookie");

    let login = http
        .get(format!(
            "{auth_base}/auth/login?__override=target-switch-b%40example.com"
        ))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_b}"))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 302);
    let authorize = login.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned();
    let callback = common::kc::authorize(&authorize, user).await;
    let cb = http
        .get(&callback)
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_b}"))
        .send()
        .await
        .unwrap();
    assert_eq!(cb.status(), 302, "no-logout switch must succeed");
    let token_c = cookie_from(&cb).expect("switch callback must set a fresh cookie");
    assert_ne!(token_b, token_c);

    let me_c = me(&http, &auth_base, &token_c).await;
    assert_eq!(
        me_c["email"].as_str().unwrap(),
        "target-switch-b@example.com"
    );
    assert_eq!(me_c["impersonator_email"].as_str().unwrap(), user);
    let stale = http
        .get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token_b}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        stale.status(),
        401,
        "the fixation guard must have revoked the presented first session"
    );
}

#[tokio::test]
#[ignore = "needs the e2e stack (run-e2e.sh)"]
async fn override_with_unknown_target_is_denied() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let user = "viewer-unknown@example.com";
    let http = client();

    let cb = login_flow(&http, &auth_base, user, Some("unknown-nobody@example.com")).await;
    assert_eq!(
        cb.status(),
        302,
        "an unknown override target must be denied"
    );
    assert_eq!(
        cb.headers()[reqwest::header::LOCATION].to_str().unwrap(),
        "/?auth_error=access_denied"
    );
    assert!(cookie_from(&cb).is_none(), "no session may be minted");
}

#[tokio::test]
#[ignore = "needs the e2e stack (run-e2e.sh)"]
async fn override_is_inert_when_disabled() {
    let auth_base = env("AUTH_BASE_DISABLED", "http://localhost:8085");
    let user = "viewer-disabled@example.com";
    let http = client();

    let cb = login_flow(&http, &auth_base, user, Some("target-disabled@example.com")).await;
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
