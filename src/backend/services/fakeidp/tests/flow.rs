//! End-to-end integration test for fakeidp: a full authorization-code + PKCE
//! login, refresh-token rotation (old token → invalid_grant), and the
//! `_control/revoke` kill path (revoked user → invalid_grant).
//!
//! The server is driven in-process on an ephemeral port via the library's
//! `app()` router, so this exercises the real HTTP handlers end to end.

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fakeidp::{AppState, Config, app, load_users};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde_json::Value;
use sha2::{Digest, Sha256};

const ISSUER: &str = "http://fakeidp.test";
const AUD: &str = "authenticator";

async fn spawn() -> String {
    let config = Config {
        issuer: ISSUER.to_string(),
        bind: "127.0.0.1:0".to_string(),
        token_ttl: 300,
        backchannel_url: None,
        default_aud: AUD.to_string(),
    };
    let state = Arc::new(AppState::new(config, load_users()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    format!("http://{addr}")
}

fn no_redirect_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn pkce_pair() -> (String, String) {
    let verifier = "test-verifier-0123456789-abcdefghijklmnopqrstuvwxyz".to_string();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn code_from_location(location: &str) -> String {
    let query = location.split_once('?').expect("redirect has a query").1;
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("code="))
        .expect("redirect carries a code")
        .to_string()
}

#[tokio::test]
async fn full_login_refresh_rotation_and_revoke() {
    let base = spawn().await;
    let client = no_redirect_client();
    let (verifier, challenge) = pkce_pair();

    // ── 1. /authorize → instant 302 with a one-time code ──────────────────
    let authz = client
        .get(format!("{base}/authorize"))
        .query(&[
            ("client_id", AUD),
            ("redirect_uri", "http://rp.test/callback"),
            ("state", "xyz"),
            ("nonce", "n1"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(authz.status().as_u16(), 302, "authorize should 302");
    let location = authz
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.contains("state=xyz"), "state echoed back");
    let code = code_from_location(&location);

    // ── 2. /token authorization_code grant → signed id_token + refresh ────
    let tok: Value = client
        .post(format!("{base}/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", "http://rp.test/callback"),
            ("client_id", AUD),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id_token = tok["id_token"].as_str().unwrap();
    let refresh1 = tok["refresh_token"].as_str().unwrap().to_string();
    assert!(!refresh1.is_empty());

    // id_token must verify against the published JWKS with the right claims.
    let jwks: Value = client
        .get(format!("{base}/jwks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let n = jwks["keys"][0]["n"].as_str().unwrap();
    let e = jwks["keys"][0]["e"].as_str().unwrap();
    let key = DecodingKey::from_rsa_components(n, e).unwrap();
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[AUD]);
    validation.set_issuer(&[ISSUER]);
    let claims = decode::<Value>(id_token, &key, &validation).unwrap().claims;
    assert_eq!(claims["email"], "alice@example.com");
    assert_eq!(claims["nonce"], "n1");
    assert!(claims["sub"].as_str().unwrap().starts_with("fakeidp|"));

    // ── 3. refresh_token grant → rotates the refresh token ────────────────
    let refreshed: Value = client
        .post(format!("{base}/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh1),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let refresh2 = refreshed["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(refresh1, refresh2, "refresh token must rotate");
    assert!(!refreshed["id_token"].as_str().unwrap().is_empty());

    // ── 4. reusing the rotated (old) refresh token → invalid_grant ────────
    let reuse = client
        .post(format!("{base}/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh1),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(
        reuse.status().as_u16(),
        400,
        "reused refresh token rejected"
    );
    let body: Value = reuse.json().await.unwrap();
    assert_eq!(body["error"], "invalid_grant");

    // ── 5. revoke the user → the current refresh token also dies ──────────
    let revoke = client
        .post(format!("{base}/_control/revoke/alice@example.com"))
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status().as_u16(), 200);

    let after_revoke = client
        .post(format!("{base}/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh2),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(
        after_revoke.status().as_u16(),
        400,
        "revoked user cannot refresh"
    );
    let body: Value = after_revoke.json().await.unwrap();
    assert_eq!(body["error"], "invalid_grant");
}
