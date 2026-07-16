//! End-to-end login loop against a running authenticator + fakeidp + Redis.
//!
//! `#[ignore]` by default (needs the stack up). Run it against docker-compose or
//! a local process stack:
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 FAKEIDP_PUBLIC=http://localhost:8084 \
//!   cargo test -p authenticator --test e2e_login_loop -- --ignored --nocapture
//! ```
//!
//! It drives the full loop: `/auth/login` -> fakeidp `/authorize` ->
//! `/auth/callback` (session + cookie) -> `/internal/authz` (JWT, verified
//! against the published JWKS) -> `/auth/me` -> `/auth/logout` -> `/internal/authz`
//! returns 401.
//!
//! Networking: in a docker-compose run the authenticator advertises fakeidp's
//! authorize URL as `http://fakeidp:8084/...`, unreachable from the host — set
//! `FAKEIDP_REWRITE_FROM=http://fakeidp:8084` and `FAKEIDP_REWRITE_TO=http://localhost:8084`
//! to rewrite it. In an all-localhost process stack no rewrite is needed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
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
    roles: Vec<String>,
    sid: String,
    aud: String,
}

#[tokio::test]
#[ignore = "requires a running authenticator + fakeidp + Redis stack"]
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
    let authorize = rewrite_host(login.headers()[reqwest::header::LOCATION].to_str().unwrap());

    // 2. Follow authorize with an explicit test user -> 302 back to the callback.
    let sep = if authorize.contains('?') { '&' } else { '?' };
    let authorized = http
        .get(format!("{authorize}{sep}user={test_user}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        authorized.status(),
        302,
        "authorize should redirect to callback"
    );
    let callback = rewrite_host(
        authorized.headers()[reqwest::header::LOCATION]
            .to_str()
            .unwrap(),
    );

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
        claims.roles.contains(&"user".to_owned()),
        "default role present"
    );
    assert!(!claims.sid.is_empty(), "stable sid present");
    let _ = &claims.tenant_id; // present (may be empty in a keyless local run)

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

    // 7. /auth/logout revokes the session and clears the cookie.
    let logout = http
        .post(format!("{auth_base}/auth/logout"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
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
