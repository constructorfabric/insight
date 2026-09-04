#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

const COOKIE: &str = "__Host-sid";
const REDIRECT_URI: &str = "http://127.0.0.1:3210/callback";
const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~";

#[derive(Deserialize)]
struct RegisteredClient {
    client_id: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
}

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn hidden_value(page: &str, name: &str) -> String {
    let marker = format!("name=\"{name}\" value=\"");
    let start = page.find(&marker).expect("hidden input must exist") + marker.len();
    let end = page[start..]
        .find('"')
        .map(|offset| start + offset)
        .expect("hidden input value must end");
    page[start..end].to_owned()
}

async fn assert_metadata(http: &common::Client, auth_base: &str) {
    for path in [
        "/.well-known/oauth-authorization-server",
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let response = http.get(format!("{auth_base}{path}")).send().await.unwrap();
        assert_eq!(response.status(), 200, "metadata endpoint {path}");
    }
}

async fn register_client(http: &common::Client, auth_base: &str) -> RegisteredClient {
    let registration = http
        .post(format!("{auth_base}/auth/oauth/register"))
        .json(&serde_json::json!({
            "client_name": "Example MCP client",
            "redirect_uris": [REDIRECT_URI],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(registration.status(), 201);
    registration.json::<RegisteredClient>().await.unwrap()
}

async fn authorize(
    http: &common::Client,
    auth_base: &str,
    test_user: &str,
    client_id: &str,
    resource: &str,
) -> String {
    let session = common::kc::login(http, auth_base, test_user).await;
    let csrf = http
        .get(format!("{auth_base}/auth/csrf"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={session}"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let challenge = B64.encode(Sha256::digest(VERIFIER.as_bytes()));
    let consent = http
        .get(format!("{auth_base}/auth/oauth/authorize"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={session}"))
        .query(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", REDIRECT_URI),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", "oauth-e2e-state"),
            ("resource", resource),
            ("scope", "mcp:query"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(consent.status(), 200);
    let request_id = hidden_value(&consent.text().await.unwrap(), "request_id");

    let decision = http
        .post(format!("{auth_base}/auth/oauth/decision"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={session}"))
        .header("X-CSRF-Token", &csrf)
        .form(&[("request_id", request_id.as_str()), ("decision", "approve")])
        .send()
        .await
        .unwrap();
    assert_eq!(decision.status(), 200);
    let redirect_to = decision.json::<serde_json::Value>().await.unwrap()["redirect_to"]
        .as_str()
        .unwrap()
        .to_owned();
    let redirect = reqwest::Url::parse(&redirect_to).unwrap();
    redirect
        .query_pairs()
        .find_map(|(name, value)| (name == "code").then(|| value.into_owned()))
        .expect("approval redirect must carry an authorization code")
}

async fn exchange_code(
    http: &common::Client,
    auth_base: &str,
    client_id: &str,
    resource: &str,
    code: &str,
) -> TokenResponse {
    let response = http
        .post(format!("{auth_base}/auth/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", VERIFIER),
            ("resource", resource),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json::<TokenResponse>().await.unwrap()
}

async fn refresh_token(
    http: &common::Client,
    auth_base: &str,
    client_id: &str,
    resource: &str,
    refresh_token: &str,
) -> reqwest::Response {
    http.post(format!("{auth_base}/auth/oauth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("resource", resource),
        ])
        .send()
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn oauth_lifecycle_issues_rotates_and_revokes_tokens() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let resource = format!("{auth_base}/mcp");
    let http = common::client();

    assert_metadata(&http, &auth_base).await;
    let client = register_client(&http, &auth_base).await;
    let code = authorize(&http, &auth_base, &test_user, &client.client_id, &resource).await;
    let issued = exchange_code(&http, &auth_base, &client.client_id, &resource, &code).await;
    assert!(!issued.access_token.is_empty());

    let refreshed = refresh_token(
        &http,
        &auth_base,
        &client.client_id,
        &resource,
        &issued.refresh_token,
    )
    .await;
    assert_eq!(refreshed.status(), 200);
    let refreshed = refreshed.json::<TokenResponse>().await.unwrap();
    assert!(!refreshed.access_token.is_empty());
    assert_ne!(refreshed.refresh_token, issued.refresh_token);

    let revoked = http
        .post(format!("{auth_base}/auth/oauth/revoke"))
        .form(&[("token", refreshed.refresh_token.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 200);

    let after_revoke = refresh_token(
        &http,
        &auth_base,
        &client.client_id,
        &resource,
        &refreshed.refresh_token,
    )
    .await;
    assert_eq!(after_revoke.status(), 400);
}
