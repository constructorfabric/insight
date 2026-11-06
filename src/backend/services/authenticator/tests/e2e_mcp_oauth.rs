#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use redis::AsyncCommands as _;
use redis::aio::MultiplexedConnection;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

const COOKIE: &str = "__Host-sid";
const REDIRECT_URI: &str = "http://127.0.0.1:3210/callback";
const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~";
const CLIENT_INDEX_KEY: &str = "{mcp_oauth}:v2:clients";
const PENDING_CLIENT_TTL_SECS: i64 = 24 * 60 * 60;
const ACTIVE_CLIENT_MIN_TTL_SECS: i64 = 364 * 24 * 60 * 60;

#[derive(Deserialize)]
struct RegisteredClient {
    client_id: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
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
        "/.well-known/oauth-protected-resource/mcp/v3",
    ] {
        let response = http.get(format!("{auth_base}{path}")).send().await.unwrap();
        assert_eq!(response.status(), 200, "metadata endpoint {path}");
    }

    // The two MCP servers are told apart by what their metadata claims: a
    // client that discovered the authoring one must not be handed the
    // read-only scope, or authoring would be granted by whichever it found.
    let custom: serde_json::Value = http
        .get(format!(
            "{auth_base}/.well-known/oauth-protected-resource/mcp/v3"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(
        custom["resource"]
            .as_str()
            .is_some_and(|resource| resource.ends_with("/mcp/v3")),
        "{custom}"
    );
    assert_eq!(
        custom["scopes_supported"],
        serde_json::json!(["mcp:author"])
    );
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
) -> (String, String, String) {
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
    let code = redirect
        .query_pairs()
        .find_map(|(name, value)| (name == "code").then(|| value.into_owned()))
        .expect("approval redirect must carry an authorization code");
    (code, session, csrf)
}

async fn exchange_code_response(
    http: &common::Client,
    auth_base: &str,
    client_id: &str,
    resource: &str,
    code: &str,
) -> reqwest::Response {
    http.post(format!("{auth_base}/auth/oauth/token"))
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
        .unwrap()
}

async fn exchange_code(
    http: &common::Client,
    auth_base: &str,
    client_id: &str,
    resource: &str,
    code: &str,
) -> TokenResponse {
    let response = exchange_code_response(http, auth_base, client_id, resource, code).await;
    assert_eq!(response.status(), 200);
    response.json::<TokenResponse>().await.unwrap()
}

fn client_key(client_id: &str) -> String {
    format!("{{mcp_oauth}}:client:{client_id}")
}

async fn client_record_ttl(redis: &mut MultiplexedConnection, client_id: &str) -> i64 {
    redis.ttl(client_key(client_id)).await.unwrap()
}

async fn client_is_counted(redis: &mut MultiplexedConnection, client_id: &str) -> bool {
    let score: Option<f64> = redis.zscore(CLIENT_INDEX_KEY, client_id).await.unwrap();
    score.is_some()
}

async fn assert_client_is_active(redis: &mut MultiplexedConnection, client_id: &str) {
    let ttl = client_record_ttl(redis, client_id).await;
    assert!(
        ttl > ACTIVE_CLIENT_MIN_TTL_SECS,
        "an exchanged client must be kept for a year, record TTL was {ttl} s"
    );
    assert!(
        client_is_counted(redis, client_id).await,
        "an exchanged client must be counted in {CLIENT_INDEX_KEY}"
    );
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
    let (code, session, csrf) =
        authorize(&http, &auth_base, &test_user, &client.client_id, &resource).await;
    let issued = exchange_code(&http, &auth_base, &client.client_id, &resource, &code).await;
    assert!(!issued.access_token.is_empty());
    assert_eq!(issued.expires_in, 600);

    let logout = http
        .post(format!("{auth_base}/auth/logout"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={session}"))
        .header("X-CSRF-Token", csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), 200);

    let logged_out = http
        .get(format!("{auth_base}/auth/me"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={session}"))
        .send()
        .await
        .unwrap();
    assert_eq!(logged_out.status(), 401);

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

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn unused_registration_expires_within_a_day_and_is_not_counted() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let http = common::client();
    let mut redis = common::store::connect().await;

    let client = register_client(&http, &auth_base).await;

    let ttl = client_record_ttl(&mut redis, &client.client_id).await;
    assert!(
        (1..=PENDING_CLIENT_TTL_SECS).contains(&ttl),
        "an unused registration must expire within a day, record TTL was {ttl} s"
    );
    assert!(
        !client_is_counted(&mut redis, &client.client_id).await,
        "an unused registration must not be counted in {CLIENT_INDEX_KEY}"
    );
}

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn exchanged_client_is_kept_for_a_year_and_counted() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let resource = format!("{auth_base}/mcp");
    let http = common::client();
    let mut redis = common::store::connect().await;

    let client = register_client(&http, &auth_base).await;
    let (code, _, _) = authorize(&http, &auth_base, &test_user, &client.client_id, &resource).await;
    exchange_code(&http, &auth_base, &client.client_id, &resource, &code).await;

    assert_client_is_active(&mut redis, &client.client_id).await;
}

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn active_client_exchanges_another_code() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let resource = format!("{auth_base}/mcp");
    let http = common::client();
    let mut redis = common::store::connect().await;

    let client = register_client(&http, &auth_base).await;
    for _ in 0..2 {
        let (code, _, _) =
            authorize(&http, &auth_base, &test_user, &client.client_id, &resource).await;
        exchange_code(&http, &auth_base, &client.client_id, &resource, &code).await;
    }

    assert_client_is_active(&mut redis, &client.client_id).await;
}

#[tokio::test]
#[ignore = "requires a running authenticator + Keycloak + Redis stack"]
async fn code_for_an_expired_registration_is_an_invalid_grant() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let resource = format!("{auth_base}/mcp");
    let http = common::client();
    let mut redis = common::store::connect().await;

    let client = register_client(&http, &auth_base).await;
    let (code, _, _) = authorize(&http, &auth_base, &test_user, &client.client_id, &resource).await;
    let _: () = redis.del(client_key(&client.client_id)).await.unwrap();

    let response =
        exchange_code_response(&http, &auth_base, &client.client_id, &resource, &code).await;

    assert_eq!(response.status(), 400);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["error"], "invalid_grant", "{body}");
    assert!(
        !client_is_counted(&mut redis, &client.client_id).await,
        "an expired registration must not be counted in {CLIENT_INDEX_KEY}"
    );
}
