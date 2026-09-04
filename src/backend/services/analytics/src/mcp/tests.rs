use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::Body;
use axum::http::{HeaderValue, Request, Response, StatusCode};
use axum::routing::{any, get};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use p256::SecretKey;
use p256::elliptic_curve::Generate as _;
use p256::elliptic_curve::sec1::ToSec1Point as _;
use p256::pkcs8::{EncodePrivateKey as _, LineEnding};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt as _;

use crate::config::SqlApiConfig;
use crate::sql_explorer::executor::{MAX_RESULT_BYTES, MAX_SQL_BYTES, QueryExecutor, QueryFailure};
use crate::sql_explorer::{start, validate_config};

use super::{
    McpConfig, QuerySqlRequest, SqlExplorer, TokenVerifier, bearer_token, validate_public_url,
};

struct SigningMaterial {
    key: EncodingKey,
    jwks: Value,
}

fn signing_material() -> anyhow::Result<SigningMaterial> {
    let secret = SecretKey::generate();
    let pem = secret.to_pkcs8_pem(LineEnding::LF)?;
    let key = EncodingKey::from_ec_pem(pem.as_bytes())?;
    let point = secret.public_key().to_sec1_point(false);
    let x = point
        .x()
        .ok_or_else(|| anyhow::anyhow!("public key has no x coordinate"))?;
    let y = point
        .y()
        .ok_or_else(|| anyhow::anyhow!("public key has no y coordinate"))?;

    Ok(SigningMaterial {
        key,
        jwks: json!({
            "keys": [{
                "kty": "EC",
                "crv": "P-256",
                "use": "sig",
                "alg": "ES256",
                "kid": "test-key",
                "x": B64.encode(x),
                "y": B64.encode(y),
            }]
        }),
    })
}

fn claims(issuer: &str) -> anyhow::Result<Value> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    Ok(json!({
        "sub": "test-user",
        "tenant_id": "test-tenant",
        "roles": "user admin",
        "sub_type": "user",
        "sid": "test-session",
        "iss": issuer,
        "aud": format!("{issuer}/mcp"),
        "scope": "openid mcp:query",
        "iat": now,
        "exp": now + 600,
        "jti": "test-token",
    }))
}

fn sign(material: &SigningMaterial, claims: &Value) -> anyhow::Result<String> {
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some("test-key".to_owned());

    Ok(encode(&header, claims, &material.key)?)
}

async fn spawn_app(app: axum::Router) -> anyhow::Result<(String, JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok((format!("http://{address}"), server))
}

async fn spawn_clickhouse(
    status: StatusCode,
    body: Vec<u8>,
) -> anyhow::Result<(SqlExplorer, JoinHandle<()>)> {
    let app = axum::Router::new().fallback(any(move || {
        let body = body.clone();
        async move {
            Response::builder()
                .status(status)
                .body(Body::from(body))
                .unwrap_or_else(|_| Response::new(Body::empty()))
        }
    }));
    let (url, server) = spawn_app(app).await?;
    let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "gold"));

    Ok((SqlExplorer::new(QueryExecutor::new(client, 1)), server))
}

fn mcp_config() -> McpConfig {
    McpConfig {
        enabled: true,
        bind_addr: "127.0.0.1:0".to_owned(),
        public_url: "http://localhost:3000".to_owned(),
        allow_insecure_private_network: false,
        max_concurrent_queries: 2,
        clickhouse_user: "test_mcp".to_owned(),
        clickhouse_password: SecretString::from("test-password".to_owned()),
    }
}

fn assert_tool_error_contains(result: &rmcp::model::CallToolResult, expected: &str) {
    assert_eq!(result.is_error, Some(true));
    let encoded = serde_json::to_string(result).unwrap_or_default();
    assert!(
        encoded.contains(expected),
        "expected {expected:?} in {encoded}"
    );
}

#[tokio::test]
async fn query_sql_returns_structured_clickhouse_json() -> anyhow::Result<()> {
    let body = serde_json::to_vec(&json!({
        "meta": [{"name": "answer", "type": "UInt8"}],
        "data": [{"answer": 1}],
        "rows": 1
    }))?;
    let (explorer, server) = spawn_clickhouse(StatusCode::OK, body).await?;

    let result = explorer
        .run_query_sql("SELECT 1 AS answer".to_owned())
        .await;

    assert_eq!(result.is_error, Some(false));
    assert_eq!(
        result
            .structured_content
            .as_ref()
            .and_then(|value| value["row_count"].as_u64()),
        Some(1)
    );
    assert_eq!(
        result
            .structured_content
            .as_ref()
            .map(|value| &value["rows"][0]["answer"]),
        Some(&json!(1))
    );
    let info = rmcp::ServerHandler::get_info(&explorer);
    assert_eq!(info.server_info.name, "insight-sql-explorer");
    let direct = explorer
        .query_sql(rmcp::handler::server::wrapper::Parameters(
            QuerySqlRequest {
                sql: "SELECT 1 AS answer".to_owned(),
            },
        ))
        .await;
    assert_eq!(direct.is_error, Some(false));
    server.abort();
    Ok(())
}

#[tokio::test]
async fn query_sql_rejects_invalid_and_oversized_input() {
    let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
        "http://127.0.0.1:1",
        "gold",
    ));
    let explorer = SqlExplorer::new(QueryExecutor::new(client, 1));

    let invalid = explorer
        .run_query_sql("DROP TABLE gold.events".to_owned())
        .await;
    assert_tool_error_contains(&invalid, "SELECT or WITH");
    let oversized = explorer.run_query_sql("x".repeat(MAX_SQL_BYTES + 1)).await;
    assert_tool_error_contains(&oversized, "request limit");
}

#[tokio::test]
async fn query_execution_reports_busy_backend_and_response_failures() -> anyhow::Result<()> {
    let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
        "http://127.0.0.1:1",
        "gold",
    ));
    let busy = SqlExplorer::new(QueryExecutor::new(client, 0))
        .executor
        .execute("SELECT 1")
        .await;
    assert!(matches!(busy, Err(QueryFailure::Busy)));

    let (invalid, invalid_server) = spawn_clickhouse(StatusCode::OK, b"not-json".to_vec()).await?;
    assert!(matches!(
        invalid.executor.execute("SELECT 1").await,
        Err(QueryFailure::InvalidResponse(_))
    ));
    invalid_server.abort();

    let (failed, failed_server) =
        spawn_clickhouse(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()).await?;
    assert!(matches!(
        failed.executor.execute("SELECT 1").await,
        Err(QueryFailure::ClickHouse(_))
    ));
    failed_server.abort();

    let (large, large_server) =
        spawn_clickhouse(StatusCode::OK, vec![b'x'; MAX_RESULT_BYTES + 1]).await?;
    assert!(matches!(
        large.executor.execute("SELECT 1").await,
        Err(QueryFailure::ResultTooLarge)
    ));
    large_server.abort();
    Ok(())
}

#[test]
fn configuration_requires_a_complete_safe_origin_and_credentials() {
    let mut config = mcp_config();
    assert!(validate_config(&config, &SqlApiConfig::default()).is_ok());
    assert!(validate_config(&McpConfig::default(), &SqlApiConfig::default()).is_ok());

    config.bind_addr = "not-an-address".to_owned();
    assert!(validate_config(&config, &SqlApiConfig::default()).is_err());
    config = mcp_config();
    config.max_concurrent_queries = 0;
    assert!(validate_config(&config, &SqlApiConfig::default()).is_err());
    config = mcp_config();
    config.clickhouse_user = "  ".to_owned();
    assert!(validate_config(&config, &SqlApiConfig::default()).is_err());
    config = mcp_config();
    config.clickhouse_password = SecretString::from(String::new());
    assert!(validate_config(&config, &SqlApiConfig::default()).is_err());

    assert!(validate_public_url("https://insight.example", false).is_ok());
    assert!(validate_public_url("http://127.0.0.1:3000", false).is_ok());
    assert!(validate_public_url("http://10.0.0.2:3000", true).is_ok());
    for invalid in [
        "http://insight.example",
        "https://user@insight.example",
        "https://insight.example/path",
        "https://insight.example?query=true",
        "https://insight.example#fragment",
    ] {
        assert!(
            validate_public_url(invalid, false).is_err(),
            "should reject {invalid}"
        );
    }
}

#[tokio::test]
async fn start_accepts_valid_configuration_and_cancellation() -> anyhow::Result<()> {
    let cancellation = CancellationToken::new();

    start(
        &mcp_config(),
        &SqlApiConfig::default(),
        "http://127.0.0.1:1",
        "gold",
        cancellation.clone(),
    )
    .await?;
    cancellation.cancel();
    tokio::task::yield_now().await;
    Ok(())
}

#[tokio::test]
async fn verifier_accepts_admin_tokens_and_rejects_other_claims() -> anyhow::Result<()> {
    let material = signing_material()?;
    let app = axum::Router::new().route(
        "/.well-known/jwks.json",
        get({
            let jwks = material.jwks.clone();
            move || {
                let jwks = jwks.clone();
                async move { Json(jwks) }
            }
        }),
    );
    let (issuer, server) = spawn_app(app).await?;
    let verifier = TokenVerifier::new(&issuer, false)?;

    let valid = sign(&material, &claims(&issuer)?)?;
    assert!(verifier.verify(&valid).await.is_ok());

    let mut insufficient = claims(&issuer)?;
    insufficient["roles"] = json!("user");
    let insufficient = sign(&material, &insufficient)?;
    assert!(matches!(
        verifier.verify(&insufficient).await,
        Err(super::AuthFailure::InsufficientScope)
    ));

    let mut future_issued = claims(&issuer)?;
    future_issued["iat"] = json!(future_issued["exp"].as_u64().unwrap_or_default() + 1);
    let future_issued = sign(&material, &future_issued)?;
    assert!(matches!(
        verifier.verify(&future_issued).await,
        Err(super::AuthFailure::InsufficientScope)
    ));

    assert!(matches!(
        verifier.verify("not-a-token").await,
        Err(super::AuthFailure::Unauthorized)
    ));
    let token_without_kid = encode(
        &Header::new(Algorithm::ES256),
        &claims(&issuer)?,
        &material.key,
    )?;
    assert!(matches!(
        verifier.verify(&token_without_kid).await,
        Err(super::AuthFailure::Unauthorized)
    ));
    let wrong_algorithm = encode(
        &Header::new(Algorithm::HS256),
        &claims(&issuer)?,
        &EncodingKey::from_secret(b"test-secret"),
    )?;
    assert!(matches!(
        verifier.verify(&wrong_algorithm).await,
        Err(super::AuthFailure::Unauthorized)
    ));
    server.abort();
    Ok(())
}

#[tokio::test]
async fn auth_middleware_returns_oauth_challenges() -> anyhow::Result<()> {
    let material = signing_material()?;
    let app = axum::Router::new().route(
        "/.well-known/jwks.json",
        get({
            let jwks = material.jwks.clone();
            move || {
                let jwks = jwks.clone();
                async move { Json(jwks) }
            }
        }),
    );
    let (issuer, server) = spawn_app(app).await?;
    let verifier = TokenVerifier::new(&issuer, false)?;
    let protected = axum::Router::new()
        .route("/", get(|| async { StatusCode::NO_CONTENT }))
        .layer(axum::middleware::from_fn_with_state(
            verifier.clone(),
            super::authenticate,
        ));

    let missing = protected
        .clone()
        .oneshot(Request::get("/").body(Body::empty())?)
        .await?;
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    assert!(
        missing
            .headers()
            .contains_key(axum::http::header::WWW_AUTHENTICATE)
    );

    let valid = sign(&material, &claims(&issuer)?)?;
    let allowed = protected
        .clone()
        .oneshot(
            Request::get("/")
                .header("authorization", format!("Bearer {valid}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(allowed.status(), StatusCode::NO_CONTENT);

    let mut claims = claims(&issuer)?;
    claims["scope"] = json!("openid");
    let denied = sign(&material, &claims)?;
    let denied = protected
        .oneshot(
            Request::get("/")
                .header("authorization", format!("Bearer {denied}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    server.abort();
    Ok(())
}

#[test]
fn bearer_tokens_are_strictly_parsed() {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer token"
            .parse()
            .unwrap_or(HeaderValue::from_static("")),
    );
    assert_eq!(bearer_token(&headers), Some("token"));

    for value in ["bearer token", "Bearer ", "Bearer two tokens"] {
        headers.insert(
            axum::http::header::AUTHORIZATION,
            value.parse().unwrap_or(HeaderValue::from_static("")),
        );
        assert_eq!(bearer_token(&headers), None);
    }
}

#[tokio::test]
async fn concurrent_jwks_misses_share_one_refresh() -> anyhow::Result<()> {
    let requests = Arc::new(AtomicUsize::new(0));
    let app = axum::Router::new().route(
        "/.well-known/jwks.json",
        get({
            let requests = requests.clone();
            move || {
                let requests = requests.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({ "keys": [] }))
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let verifier = TokenVerifier::new(&format!("http://{address}"), false)?;

    let (first, second) = tokio::join!(verifier.cached_jwks(), verifier.cached_jwks());

    assert!(first.is_ok());
    assert!(second.is_ok());
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert!(verifier.refresh_jwks(Some("missing-key")).await.is_ok());
    assert!(verifier.refresh_jwks(Some("missing-key")).await.is_ok());
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    server.abort();
    Ok(())
}

#[test]
fn backend_failures_have_a_generic_public_message() {
    let clickhouse_failure = QueryFailure::ClickHouse(clickhouse::error::Error::BadResponse(
        "private backend diagnostic".to_owned(),
    ));
    let parse_error = serde_json::Error::io(std::io::Error::other("private JSON diagnostic"));
    let json_failure = QueryFailure::InvalidResponse(parse_error);

    assert_eq!(
        clickhouse_failure.public_message(),
        "query execution failed"
    );
    assert_eq!(json_failure.public_message(), "query execution failed");
    assert!(
        !clickhouse_failure
            .public_message()
            .contains("private backend diagnostic")
    );
    assert!(!json_failure.public_message().contains("JSON"));
}

#[test]
fn bounded_query_failures_have_stable_public_messages() {
    assert_eq!(
        QueryFailure::Busy.public_message(),
        "the SQL explorer is busy; retry shortly"
    );
    assert_eq!(
        QueryFailure::ResultTooLarge.public_message(),
        "query result exceeded the response limit"
    );
    assert_eq!(QueryFailure::Timeout.public_message(), "query timed out");
}
