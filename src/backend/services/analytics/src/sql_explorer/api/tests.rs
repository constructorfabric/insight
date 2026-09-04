use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::routing::post;
use secrecy::SecretString;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use super::*;

const TOKEN: &str = "synthetic-test-token-not-a-real-secret";
type R = Result<(), Box<dyn std::error::Error>>;

fn app(url: &str, capacity: usize) -> Router {
    let config = SqlApiConfig {
        enabled: true,
        token: SecretString::from(TOKEN.to_owned()),
    };
    let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "gold"));
    router(&config, QueryExecutor::new(client, capacity))
}

async fn send(
    app: Router,
    token: Option<&str>,
    body: &str,
) -> Result<Response, Box<dyn std::error::Error>> {
    let mut request = Request::post("/api/sql/query").header(CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(AUTHORIZATION, token);
    }
    Ok(app
        .oneshot(request.body(Body::from(body.to_owned()))?)
        .await?)
}

#[tokio::test]
async fn invalid_credentials_never_execute_queries() -> R {
    for token in [
        None,
        Some("Bearer wrong"),
        Some("Basic wrong"),
        Some("Bearer "),
        Some("Bearer two tokens"),
    ] {
        let response = send(app("http://127.0.0.1:1", 1), token, "{}").await?;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "credential: {token:?}"
        );
        assert_eq!(response.headers()[WWW_AUTHENTICATE], "Bearer");
        assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
    }
    Ok(())
}

#[tokio::test]
async fn request_and_sql_validation_precede_database_calls() -> R {
    let auth = format!("bEaReR {TOKEN}");
    for body in [
        "{}",
        "{",
        r#"{"sql":1}"#,
        r#"{"sql":"SELECT 1","extra":true}"#,
        r#"{"sql":"DROP TABLE gold.events"}"#,
        r#"{"sql":"SELECT 1; SELECT 2"}"#,
    ] {
        let response = send(app("http://127.0.0.1:1", 1), Some(&auth), body).await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "body: {body}");
    }
    Ok(())
}

#[tokio::test]
async fn query_returns_the_shared_result_contract() -> R {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let upstream = Router::new().route(
        "/",
        post(|| async {
            Json(json!({"meta":[{"name":"answer","type":"UInt8"}],"data":[{"answer":1}]}))
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await });
    let response = send(
        app(&format!("http://{address}"), 1),
        Some(&format!("Bearer {TOKEN}")),
        r#"{"sql":"SELECT 1 AS answer"}"#,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
    assert_eq!(
        body,
        json!({"columns":[{"name":"answer","type":"UInt8"}],"rows":[{"answer":1}],"row_count":1,"truncated":false})
    );
    server.abort();
    Ok(())
}

#[tokio::test]
async fn busy_and_backend_errors_use_generic_http_errors() -> R {
    for (capacity, expected) in [
        (0, StatusCode::TOO_MANY_REQUESTS),
        (1, StatusCode::INTERNAL_SERVER_ERROR),
    ] {
        let response = send(
            app("http://127.0.0.1:1", capacity),
            Some(&format!("Bearer {TOKEN}")),
            r#"{"sql":"SELECT 1"}"#,
        )
        .await?;
        assert_eq!(response.status(), expected);
        let bytes = to_bytes(response.into_body(), 4096).await?;
        let body = String::from_utf8(bytes.to_vec())?;
        assert!(!body.contains("127.0.0.1"));
        assert!(!body.contains(TOKEN));
    }
    Ok(())
}

#[tokio::test]
async fn oversized_requests_are_bounded() -> R {
    let body = json!({"sql":"x".repeat(MAX_REQUEST_BODY_BYTES)}).to_string();
    let response = send(
        app("http://127.0.0.1:1", 1),
        Some(&format!("Bearer {TOKEN}")),
        &body,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    Ok(())
}

#[test]
fn duplicate_authorization_headers_are_rejected() -> R {
    let mut headers = HeaderMap::new();
    headers.append(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {TOKEN}"))?,
    );
    headers.append(AUTHORIZATION, HeaderValue::from_static("Bearer wrong"));
    assert!(!valid_token(&headers, &Sha256::digest(TOKEN).into()));
    Ok(())
}
