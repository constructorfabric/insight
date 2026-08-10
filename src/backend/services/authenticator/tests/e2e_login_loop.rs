//! End-to-end login loop against a running authenticator + Keycloak + Redis
//! (`run-e2e.sh` boots the stack):
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 \
//!   cargo test -p authenticator --test e2e_login_loop -- --ignored --nocapture
//! ```
//!
//! It drives the full loop: `/auth/login` -> the realm's login form ->
//! `/auth/callback` (session + cookie) -> `/internal/authz` (JWT, verified
//! against the published JWKS) -> `/auth/me` -> `/auth/logout` -> `/internal/authz`
//! returns 401.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;

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

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}
#[derive(Deserialize)]
struct Jwk {
    x: String,
    y: String,
    #[serde(default)]
    kid: Option<String>,
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
    tenant_id: String,
    /// Space-delimited on the wire (OAuth `scope` shape — the downstream
    /// verifier's `token_scopes` mapping splits on whitespace).
    roles: String,
    sid: String,
    aud: String,
}

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn full_login_exchange_logout_loop() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let http = client();

    // 1. /auth/login -> 302 to the IdP authorize endpoint.
    let login = http
        .get(format!("{auth_base}/auth/login?return_to=/dashboard"))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 302, "login should redirect to the IdP");
    let authorize = login.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned();

    // 2. Authenticate on the realm's login form -> redirect to the callback.
    let callback = common::kc::authorize(&authorize, &test_user).await;

    // 3. Follow the callback -> session created, cookie set, 302 to return_to.
    let cb = http.get(&callback).send().await.unwrap();
    assert_eq!(
        cb.status(),
        302,
        "callback should set the cookie and redirect"
    );
    assert_eq!(
        cb.headers()[reqwest::header::LOCATION].to_str().unwrap(),
        "/dashboard",
        "callback should honor the sanitized return_to"
    );
    let token = cookie_from(&cb).expect("callback must set __Host-sid");

    // 4. /internal/authz with the cookie -> 200 + X-Gateway-Jwt + Cache-Control.
    let authz = http
        .get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(authz.status(), 200, "authz should exchange cookie for JWT");
    assert!(
        authz.headers().contains_key("cache-control"),
        "authz must emit Cache-Control"
    );
    let bearer = authz.headers()["x-gateway-jwt"]
        .to_str()
        .unwrap()
        .to_owned();
    let jwt = bearer
        .strip_prefix("Bearer ")
        .expect("X-Gateway-Jwt is a Bearer token");

    // 5. Verify the JWT against the published JWKS (ES256).
    let jwks: Jwks = http
        .get(format!("{auth_base}/.well-known/jwks.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let header = jsonwebtoken::decode_header(jwt).unwrap();
    assert_eq!(header.alg, Algorithm::ES256);
    let jwk = jwks
        .keys
        .iter()
        .find(|k| header.kid.is_none() || k.kid == header.kid)
        .expect("a JWKS key matching the token kid");
    let decoding = DecodingKey::from_ec_components(&jwk.x, &jwk.y).unwrap();
    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_audience(&["internal-services"]);
    let claims = decode::<Claims>(jwt, &decoding, &validation)
        .expect("gateway JWT verifies against the JWKS")
        .claims;
    assert!(!claims.sub.is_empty(), "JWT sub (person_id) must be set");
    assert_eq!(claims.aud, "internal-services");
    assert!(
        claims.roles.split_whitespace().any(|r| r == "user"),
        "default role present"
    );
    assert!(!claims.sid.is_empty(), "stable sid present");
    let _ = &claims.tenant_id; // present (may be empty in a keyless local run)

    // 5b. The discovery document points downstream verifiers at that JWKS
    //     (cf-gears-oidc-authn-plugin resolves jwks_uri from it).
    let discovery: serde_json::Value = http
        .get(format!("{auth_base}/.well-known/openid-configuration"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        discovery["issuer"].as_str().is_some_and(|s| !s.is_empty()),
        "discovery must carry the issuer"
    );
    assert!(
        discovery["jwks_uri"]
            .as_str()
            .is_some_and(|s| s.ends_with("/.well-known/jwks.json")),
        "discovery must point at the published JWKS"
    );

    // 6. /auth/me returns the session summary.
    let me = http
        .get(format!("{auth_base}/auth/me"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), 200);
    let me_body: serde_json::Value = me.json().await.unwrap();
    assert!(me_body.get("user").is_some());
    assert!(me_body.get("refresh_at").is_some());

    // 7. /auth/logout revokes the session and clears the cookie. State-changing
    //    /auth/* requires the CSRF token (step 10.5) — /auth/me echoed it.
    let csrf = me_body["csrf_token"].as_str().unwrap().to_owned();
    let logout = http
        .post(format!("{auth_base}/auth/logout"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .header("X-CSRF-Token", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), 200);
    let cleared = logout
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .any(|h| h.to_str().unwrap_or("").contains("Max-Age=0"));
    assert!(cleared, "logout must clear the cookie");

    // 8. The exchange now fails closed.
    let after = http
        .get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), 401, "revoked session must 401");
    assert_eq!(
        after.headers()["cache-control"].to_str().unwrap(),
        "no-store",
        "401 must never be cached"
    );
}

/// Failed callbacks bounce back into the SPA (#2032): the browser lands on
/// `/auth/callback` from an IdP redirect, so a problem+json answer would
/// dead-end the login on raw JSON. Every browser-facing failure must 302 to
/// `default_return_to` with a fixed `auth_error=<reason>` the SPA consumes to
/// restart the flow.
#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn failed_callback_redirects_into_the_spa_with_auth_error() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let http = client();

    // Distinct `state` values per case AND per run: the per-state callback
    // rate-limit bucket (5-burst, 10/min refill) must not couple these
    // requests — or trip on rapid suite re-runs against the same stack.
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let cases = [
        (
            format!("code=x&state=e2e-auth-error-unknown-{run}"),
            "/?auth_error=state_expired",
        ),
        (
            format!("error=access_denied&state=e2e-auth-error-idp-{run}"),
            "/?auth_error=idp_error",
        ),
        (
            format!("state=e2e-auth-error-no-code-{run}"),
            "/?auth_error=invalid_callback",
        ),
    ];
    for (query, expected_location) in cases {
        let resp = http
            .get(format!("{auth_base}/auth/callback?{query}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 302, "{query} must redirect");
        let location = resp.headers()[reqwest::header::LOCATION].to_str().unwrap();
        assert_eq!(location, expected_location, "for {query}");
    }
}
