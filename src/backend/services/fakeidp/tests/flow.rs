//! End-to-end integration tests for fakeidp, driving the real HTTP handlers
//! in-process on ephemeral ports via the library's `app()` router:
//!
//! * the full authorization-code + PKCE login, refresh-token rotation, and the
//!   `_control/revoke` kill path;
//! * discovery / JWKS;
//! * every `/token` and `/authorize` error branch (unknown/used code, PKCE
//!   failures, unsupported grant, missing/unknown refresh token);
//! * `/end_session`, and all four `/_control/*` hooks (revoke, back-channel
//!   with a stub RP, outage modes, state dump).
//!
//! Env-driven paths (`Config::from_env`, `run`) are covered separately in
//! `tests/boot.rs` (its own process, so it never races these on `std::env`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{Router, extract::State, routing::post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fakeidp::{AppState, Config, app, load_users};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde_json::Value;
use sha2::{Digest, Sha256};

const ISSUER: &str = "http://fakeidp.test";
const AUD: &str = "authenticator";

fn config(backchannel_url: Option<String>) -> Config {
    Config {
        issuer: ISSUER.to_string(),
        bind: "127.0.0.1:0".to_string(),
        token_ttl: 300,
        backchannel_url,
        default_aud: AUD.to_string(),
    }
}

async fn spawn_with(cfg: Config) -> String {
    let state = Arc::new(AppState::new(cfg, load_users()));
    serve(app(state)).await
}

async fn spawn() -> String {
    spawn_with(config(None)).await
}

/// Bind an ephemeral port, serve `router` in the background, return the base URL.
async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
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

/// Decode a JWT payload without verifying the signature — for tests that only
/// assert claim values.
fn unverified_claims(jwt: &str) -> Value {
    let payload = jwt.split('.').nth(1).expect("jwt has a payload segment");
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap()
}

/// Run the S256 `/authorize` (as `user`) → grab code → exchange it for tokens,
/// returning the parsed token response. Shared by several tests.
async fn login(base: &str, client: &reqwest::Client, user: &str) -> (Value, String) {
    let (verifier, challenge) = pkce_pair();
    let authz = client
        .get(format!("{base}/authorize"))
        .query(&[
            ("client_id", AUD),
            ("redirect_uri", "http://rp.test/callback"),
            ("state", "xyz"),
            ("nonce", "n1"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("user", user),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(authz.status().as_u16(), 302);
    let location = authz.headers()["location"].to_str().unwrap().to_string();
    let code = code_from_location(&location);
    let tokens: Value = client
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
    (tokens, verifier)
}

#[tokio::test]
async fn full_login_refresh_rotation_and_revoke() {
    let base = spawn().await;
    let client = no_redirect_client();

    // ── login: /authorize (302) → /token (code+PKCE) → signed id_token ────
    let (tok, _verifier) = login(&base, &client, "alice@example.com").await;
    let id_token = tok["id_token"].as_str().unwrap();
    let refresh1 = tok["refresh_token"].as_str().unwrap().to_string();
    assert_eq!(tok["token_type"], "Bearer");
    assert_eq!(tok["expires_in"], 300);
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
    // The single tenant from users.yaml is emitted for e2e to assert/map.
    assert_eq!(
        claims["tenant_id"],
        serde_json::json!("00000000-df51-5b42-9538-d2b56b7ee953")
    );

    // ── refresh rotates; the old token then fails closed ─────────────────
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

    let reuse = client
        .post(format!("{base}/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh1),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(reuse.status().as_u16(), 400);
    assert_eq!(
        reuse.json::<Value>().await.unwrap()["error"],
        "invalid_grant"
    );

    // ── revoke the user → the current (valid) refresh token also dies ─────
    let revoke = client
        .post(format!("{base}/_control/revoke/alice@example.com"))
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status().as_u16(), 200);

    let after = client
        .post(format!("{base}/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh2),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(after.status().as_u16(), 400);
    assert_eq!(
        after.json::<Value>().await.unwrap()["error"],
        "invalid_grant"
    );
}

#[tokio::test]
async fn discovery_and_jwks() {
    let base = spawn().await;
    let client = reqwest::Client::new();
    let disco: Value = client
        .get(format!("{base}/.well-known/openid-configuration"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(disco["issuer"], ISSUER);
    assert_eq!(disco["token_endpoint"], format!("{ISSUER}/token"));
    assert_eq!(disco["jwks_uri"], format!("{ISSUER}/jwks"));
    assert!(
        disco["code_challenge_methods_supported"]
            .as_array()
            .unwrap()
            .contains(&Value::from("S256"))
    );

    let jwks: Value = client
        .get(format!("{base}/jwks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(jwks["keys"][0]["kty"], "RSA");
    assert_eq!(jwks["keys"][0]["alg"], "RS256");
    assert_eq!(jwks["keys"][0]["kid"], "fakeidp-key-1");
}

#[tokio::test]
async fn authorize_selects_user_and_rejects_unknown() {
    let base = spawn().await;
    let client = no_redirect_client();

    // Explicit user selection is honoured (bob, not the default alice).
    let authz = client
        .get(format!("{base}/authorize"))
        .query(&[
            ("redirect_uri", "http://rp.test/cb?already=1"),
            ("user", "bob@example.com"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(authz.status().as_u16(), 302);
    // redirect_uri already had a query, so the code is appended with '&'.
    let loc = authz.headers()["location"].to_str().unwrap();
    assert!(
        loc.contains("already=1&code="),
        "appends with & to existing query: {loc}"
    );

    // Unknown user is denied.
    let denied = client
        .get(format!("{base}/authorize"))
        .query(&[
            ("redirect_uri", "http://rp.test/cb"),
            ("user", "nobody@example.com"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status().as_u16(), 400);
    assert_eq!(
        denied.json::<Value>().await.unwrap()["error"],
        "access_denied"
    );
}

#[tokio::test]
async fn authorize_with_plain_pkce() {
    let base = spawn().await;
    let client = no_redirect_client();
    // method=plain → challenge == verifier.
    let verifier = "plain-verifier-value";
    let authz = client
        .get(format!("{base}/authorize"))
        .query(&[
            ("redirect_uri", "http://rp.test/cb"),
            ("code_challenge", verifier),
            ("code_challenge_method", "plain"),
        ])
        .send()
        .await
        .unwrap();
    let code = code_from_location(authz.headers()["location"].to_str().unwrap());
    let resp = client
        .post(format!("{base}/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert!(
        !resp.json::<Value>().await.unwrap()["id_token"]
            .as_str()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn token_error_branches() {
    let base = spawn().await;
    let client = no_redirect_client();

    async fn err(client: &reqwest::Client, base: &str, form: &[(&str, &str)]) -> (u16, Value) {
        let r = client
            .post(format!("{base}/token"))
            .form(form)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        (status, r.json().await.unwrap())
    }

    // Missing code.
    let (s, b) = err(&client, &base, &[("grant_type", "authorization_code")]).await;
    assert_eq!((s, &b["error"]), (400, &Value::from("invalid_request")));

    // Unknown code.
    let (s, b) = err(
        &client,
        &base,
        &[("grant_type", "authorization_code"), ("code", "nope")],
    )
    .await;
    assert_eq!((s, &b["error"]), (400, &Value::from("invalid_grant")));

    // Unsupported grant type.
    let (s, b) = err(&client, &base, &[("grant_type", "password")]).await;
    assert_eq!(
        (s, &b["error"]),
        (400, &Value::from("unsupported_grant_type"))
    );

    // Missing / unknown refresh token.
    let (s, b) = err(&client, &base, &[("grant_type", "refresh_token")]).await;
    assert_eq!((s, &b["error"]), (400, &Value::from("invalid_request")));
    let (s, b) = err(
        &client,
        &base,
        &[("grant_type", "refresh_token"), ("refresh_token", "nope")],
    )
    .await;
    assert_eq!((s, &b["error"]), (400, &Value::from("invalid_grant")));

    // PKCE: challenge present but no verifier, then wrong verifier, then a
    // reused code — each fails invalid_grant.
    let (_, challenge) = pkce_pair();
    let mint = |q: Vec<(&'static str, String)>| {
        let client = client.clone();
        let base = base.clone();
        async move {
            let a = client
                .get(format!("{base}/authorize"))
                .query(&q)
                .send()
                .await
                .unwrap();
            code_from_location(a.headers()["location"].to_str().unwrap())
        }
    };

    let code = mint(vec![
        ("redirect_uri", "http://rp.test/cb".into()),
        ("code_challenge", challenge.clone()),
        ("code_challenge_method", "S256".into()),
    ])
    .await;
    // no verifier
    let (s, b) = err(
        &client,
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert_eq!((s, &b["error"]), (400, &Value::from("invalid_grant")));
    // wrong verifier (same code is still unused because PKCE is checked before
    // the code is marked used)
    let (s, _) = err(
        &client,
        &base,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", "wrong"),
        ],
    )
    .await;
    assert_eq!(s, 400);

    // Reused code: mint (no PKCE), spend it, spend it again → invalid_grant.
    let code = mint(vec![("redirect_uri", "http://rp.test/cb".into())]).await;
    let ok = client
        .post(format!("{base}/token"))
        .form(&[("grant_type", "authorization_code"), ("code", &code)])
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200);
    let (s, b) = err(
        &client,
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert_eq!((s, &b["error"]), (400, &Value::from("invalid_grant")));

    // Unsupported PKCE method.
    let code = mint(vec![
        ("redirect_uri", "http://rp.test/cb".into()),
        ("code_challenge", "x".into()),
        ("code_challenge_method", "S512".into()),
    ])
    .await;
    let (s, _) = err(
        &client,
        &base,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", "x"),
        ],
    )
    .await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn end_session_redirects_or_ok() {
    let base = spawn().await;
    let client = no_redirect_client();

    let r = client
        .get(format!("{base}/end_session"))
        .query(&[
            ("post_logout_redirect_uri", "http://rp.test/bye"),
            ("state", "s1"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 302);
    assert_eq!(r.headers()["location"], "http://rp.test/bye?state=s1");

    // No redirect uri → 200.
    let r = client
        .post(format!("{base}/end_session"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
}

#[tokio::test]
async fn outage_modes_and_reset() {
    let base = spawn().await;
    let client = reqwest::Client::new();

    // Invalid mode rejected.
    let bad = client
        .post(format!("{base}/_control/outage"))
        .json(&serde_json::json!({"mode": "boom"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status().as_u16(), 400);

    // 5xx mode → /token returns 503.
    client
        .post(format!("{base}/_control/outage"))
        .json(&serde_json::json!({"mode": "5xx"}))
        .send()
        .await
        .unwrap();
    let during = client
        .post(format!("{base}/token"))
        .form(&[("grant_type", "refresh_token"), ("refresh_token", "x")])
        .send()
        .await
        .unwrap();
    assert_eq!(during.status().as_u16(), 503);

    // Back off → /token works again (unknown token → 400, i.e. not 503).
    client
        .post(format!("{base}/_control/outage"))
        .json(&serde_json::json!({"mode": "off"}))
        .send()
        .await
        .unwrap();
    let after = client
        .post(format!("{base}/token"))
        .form(&[("grant_type", "refresh_token"), ("refresh_token", "x")])
        .send()
        .await
        .unwrap();
    assert_eq!(after.status().as_u16(), 400);
}

#[tokio::test]
async fn control_state_and_revoke_unknown() {
    let base = spawn().await;
    // No-redirect client: /authorize 302s to an unreachable rp.test, and we
    // only care about the mint side effect, not following the redirect.
    let client = no_redirect_client();

    // Mint a code so the state dump has something to show.
    let _ = client
        .get(format!("{base}/authorize"))
        .query(&[("redirect_uri", "http://rp.test/cb")])
        .send()
        .await
        .unwrap();

    let state: Value = client
        .get(format!("{base}/_control/state"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        state["users"]
            .as_array()
            .unwrap()
            .contains(&Value::from("alice@example.com"))
    );
    assert_eq!(state["outage"], "off");
    assert_eq!(state["codes"].as_array().unwrap().len(), 1);

    // Revoking an unknown user is a 404.
    let r = client
        .post(format!("{base}/_control/revoke/ghost@example.com"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);
}

#[tokio::test]
async fn backchannel_hook() {
    let client = reqwest::Client::new();

    // Not configured → 412.
    let base = spawn().await;
    let r = client
        .post(format!("{base}/_control/backchannel/alice@example.com"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 412);

    // Unknown user → 404 (before the config check).
    let r = client
        .post(format!("{base}/_control/backchannel/ghost@example.com"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);

    // Configured with a stub RP that captures the logout_token.
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let rp = Router::new()
        .route("/bcl", post(rp_receiver))
        .with_state(captured.clone());
    let rp_base = serve(rp).await;

    let base = spawn_with(config(Some(format!("{rp_base}/bcl")))).await;
    let ok = client
        .post(format!("{base}/_control/backchannel/alice@example.com"))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200);
    assert_eq!(ok.json::<Value>().await.unwrap()["rp_status"], 200);

    // The RP received a signed logout_token with the back-channel events claim.
    let token = captured.lock().unwrap()[0].clone();
    let payload = token.split('.').nth(1).unwrap();
    let json: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
    assert_eq!(json["sid"], "sid-alice-0001");
    assert!(
        json["events"]
            .as_object()
            .unwrap()
            .contains_key("http://schemas.openid.net/event/backchannel-logout")
    );

    // Configured but the RP is unreachable → 502.
    let base = spawn_with(config(Some("http://127.0.0.1:1/nope".into()))).await;
    let bad = client
        .post(format!("{base}/_control/backchannel/alice@example.com"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status().as_u16(), 502);
}

#[tokio::test]
async fn refresh_preserves_non_default_audience() {
    let base = spawn().await;
    let client = no_redirect_client();

    // Log in with a non-default client_id (no PKCE, for brevity).
    let authz = client
        .get(format!("{base}/authorize"))
        .query(&[
            ("client_id", "spa-client"),
            ("redirect_uri", "http://rp.test/cb"),
            ("user", "bob@example.com"),
        ])
        .send()
        .await
        .unwrap();
    let code = code_from_location(authz.headers()["location"].to_str().unwrap());
    let tok: Value = client
        .post(format!("{base}/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", "spa-client"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        unverified_claims(tok["id_token"].as_str().unwrap())["aud"],
        "spa-client"
    );

    // The rotated ID token must keep the original client's audience, not fall
    // back to the default.
    let refresh = tok["refresh_token"].as_str().unwrap();
    let refreshed: Value = client
        .post(format!("{base}/token"))
        .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let claims = unverified_claims(refreshed["id_token"].as_str().unwrap());
    assert_eq!(
        claims["aud"], "spa-client",
        "audience preserved across refresh"
    );
    assert_eq!(
        claims["tenant_id"],
        serde_json::json!("00000000-df51-5b42-9538-d2b56b7ee953"),
        "bob's tenant present on refreshed token too"
    );
}

async fn rp_receiver(
    State(store): State<Arc<Mutex<Vec<String>>>>,
    axum::extract::Form(form): axum::extract::Form<HashMap<String, String>>,
) -> &'static str {
    store
        .lock()
        .unwrap()
        .push(form.get("logout_token").cloned().unwrap_or_default());
    "ok"
}
