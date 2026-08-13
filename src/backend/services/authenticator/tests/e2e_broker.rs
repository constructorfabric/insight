//! End-to-end GitHub-brokered login against a running authenticator +
//! Keycloak + Redis stack (#2163 scenario 8; `run-e2e.sh` boots it).
//!
//! The realm registers the `github` identity provider in the documented
//! shape (deploy/gitops/README.md, "Enabling GitHub sign-in"): `trustEmail`,
//! the hardcoded tenant pin, and the GitHub-id -> `idp_sub` mapper — aimed
//! at the rig-local GitHub stub (tests/github-stub.py) in place of
//! github.com, so the REAL social importer runs with no real GitHub leg.
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 E2E_GITHUB_TENANT_PIN=<uuid> \
//!   cargo test -p authenticator --test e2e_broker -- --ignored --nocapture
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use std::time::{Duration, Instant};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;

const COOKIE: &str = "__Host-sid";
// github-stub.py's `known` identity: resolvable by the identity stub.
const KNOWN_EMAIL: &str = "broker-known@example.com";
const KNOWN_GITHUB_ID: &str = "7100001";

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
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
}

async fn me(http: &common::Client, auth_base: &str, token: &str) -> serde_json::Value {
    let resp = http
        .get(format!("{auth_base}/auth/me"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "/auth/me must answer for a live session"
    );
    resp.json().await.unwrap()
}

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn brokered_login_pins_tenant_and_stamps_idp_sub() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let pin = std::env::var("E2E_GITHUB_TENANT_PIN")
        .expect("E2E_GITHUB_TENANT_PIN must carry the registration's hardcoded tenant");
    let http = common::client();

    // 1. The full brokered hop chain ends in a session cookie.
    let cb = common::kc::broker_login_github(&http, &auth_base, "known").await;
    assert_eq!(
        cb.status(),
        302,
        "callback must set the cookie and redirect"
    );
    let token = common::kc::session_cookie(&cb).expect("brokered callback must set __Host-sid");

    // 2. The gateway JWT carries the pinned tenant — a single string, the
    //    registration's value, not the roster realm's default tenant.
    let authz = http
        .get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(authz.status(), 200, "authz should exchange cookie for JWT");
    let bearer = authz.headers()["x-gateway-jwt"]
        .to_str()
        .unwrap()
        .to_owned();
    let jwt = bearer
        .strip_prefix("Bearer ")
        .expect("X-Gateway-Jwt is a Bearer token");

    let jwks: Jwks = http
        .get(format!("{auth_base}/.well-known/jwks.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let header = jsonwebtoken::decode_header(jwt).unwrap();
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
    assert_eq!(
        claims.tenant_id, pin,
        "the token tenant must be the registration's pin"
    );

    // 3. The broker recorded a realm-local user linked to the GitHub
    //    identity, with the email trusted (verified) — the linking
    //    mechanism, not an access grant.
    let user = common::kc::find_user(KNOWN_EMAIL).await;
    assert_eq!(
        user["emailVerified"],
        serde_json::json!(true),
        "trustEmail must mark the brokered email verified"
    );
    let links = common::kc::federated_identity(KNOWN_EMAIL).await;
    assert_eq!(
        links[0]["identityProvider"],
        serde_json::json!("github"),
        "the realm user must be linked to the github registration"
    );
    assert_eq!(
        links[0]["userId"],
        serde_json::json!(KNOWN_GITHUB_ID),
        "the link must carry the GitHub numeric id"
    );

    // 3b. The documented claim contract, read straight off the IdP: a
    //     returning brokered login's id_token carries the pinned tenant as
    //     a single string and the GitHub id as idp_sub. (KC 26 hides
    //     unmanaged user attributes from admin-API reads, so the token is
    //     the observable proof both identity-provider mappers stamped.)
    //     The rig's own PKCE-free authorize request, so the code exchanges.
    let redirect_uri = format!("{auth_base}/auth/callback");
    let callback = common::kc::broker_direct_callback_url(&redirect_uri, "known").await;
    let code = reqwest::Url::parse(&callback)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "code")
        .expect("the brokered callback must carry a code")
        .1
        .into_owned();
    let id_claims = common::kc::exchange_code_for_id_claims(&code, &redirect_uri).await;
    assert_eq!(
        id_claims["tenant_id"],
        serde_json::json!(pin),
        "the id_token tenant must be the pin, a single string"
    );
    assert_eq!(
        id_claims["idp_sub"],
        serde_json::json!(KNOWN_GITHUB_ID),
        "the id_token must carry the GitHub id as idp_sub"
    );

    // 4. The brokered session rides the standard refresh lifecycle: the
    //    planned refresh moves once rotation happens, and the session
    //    survives it. (Realm lifespan 15s, refresh due ~5s after mint.)
    let first_refresh_at = me(&http, &auth_base, &token).await["refresh_at"].clone();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if me(&http, &auth_base, &token).await["refresh_at"] != first_refresh_at {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the brokered session's IdP tokens never rotated"
        );
    }
    let authz = http
        .get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(authz.status(), 200, "the session must survive the rotation");
}

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn brokered_unknown_email_is_refused() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let http = common::client();

    // The stub's `unknown` identity completes the GitHub leg, but its email
    // resolves to nobody — the unknown-person refusal, not an auto-created
    // account.
    let cb = common::kc::broker_login_github(&http, &auth_base, "unknown").await;
    assert_eq!(
        cb.status(),
        302,
        "the refusal is a redirect, not an error page"
    );
    let location = cb.headers()[reqwest::header::LOCATION].to_str().unwrap();
    assert!(
        location.contains("auth_error=access_denied"),
        "the refusal must land with the auth_error reason, got {location}"
    );
    assert!(
        common::kc::session_cookie(&cb).is_none(),
        "a refused brokered login must not set {COOKIE}"
    );
}
