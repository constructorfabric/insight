use std::convert::Infallible;
use std::sync::Arc;
use std::task::Poll;

use axum::body::{Body, to_bytes};
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderMap, HeaderValue, Request};
use chrono::{DateTime, Utc};
use clickhouse::test::{Mock, handlers, status};
use futures::stream;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use toolkit::api::{OpenApiInfo, OpenApiRegistryImpl};
use tower::ServiceExt as _;
use uuid::Uuid;

use super::*;
use crate::api::AppState;
use crate::api::admission::{
    INGEST_TOKEN_HEADER, IngestAdmission, MAX_CONCURRENT_WRITES, MAX_REQUEST_BODY_BYTES,
    TokenVerifier,
};
use crate::chat::ChatClient;
use crate::definitions::Definitions;
use crate::definitions::memory::MemoryDefinitions;
use crate::metric_query::MetricRunner;
use crate::raw_data::RawDataStore;
use crate::tables::TableStore;

const TEST_TOKEN: &str = "correct-token-0123456789abcdefghi";

#[derive(Debug, Deserialize, clickhouse::Row)]
struct CapturedRawDataRow {
    #[serde(with = "clickhouse::serde::uuid")]
    id: Uuid,
    table_name: String,
    raw_data: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    received_at: DateTime<Utc>,
}

fn verifier() -> TokenVerifier {
    TokenVerifier::new(&SecretString::from(TEST_TOKEN.to_owned()))
}

fn state(mock: &Mock) -> Arc<AppState> {
    let url = mock.url();
    let definitions: Arc<dyn Definitions> = Arc::new(MemoryDefinitions::new());
    Arc::new(AppState::new(
        RawDataStore::new(insight_clickhouse::Client::new(
            insight_clickhouse::Config::new(url, "insight"),
        )),
        TableStore::new(insight_clickhouse::Client::new(
            insight_clickhouse::Config::new(url, "insight"),
        )),
        definitions.clone(),
        MetricRunner::new(
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "insight")),
            crate::metric_query::People::new("identity"),
        ),
        ChatClient::keyless(),
        crate::identity::IdentityClient::fixed(true),
        crate::catalog::Catalog::new(
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                "http://catalogue.invalid",
                "insight",
            )),
            "insight".to_owned(),
        ),
    ))
}

fn app(mock: &Mock) -> Router {
    let openapi = OpenApiRegistryImpl::new();

    register_routes(
        Router::new(),
        &openapi,
        state(mock),
        IngestAdmission::new(&SecretString::from(TEST_TOKEN.to_owned())),
    )
}

fn post(body: Body, token: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/v1/raw-data")
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header(INGEST_TOKEN_HEADER, token);
    }

    request
        .body(body)
        .unwrap_or_else(|error| panic!("test request must be valid: {error}"))
}

fn unpollable_body() -> Body {
    Body::from_stream(stream::poll_fn(
        |_| -> Poll<Option<Result<String, Infallible>>> {
            panic!("admission middleware must not poll the request body")
        },
    ))
}

#[test]
fn configured_instance_token_is_accepted() {
    let mut headers = HeaderMap::new();
    headers.insert(INGEST_TOKEN_HEADER, HeaderValue::from_static(TEST_TOKEN));

    assert!(verifier().authorizes(&headers));
}

#[test]
fn malformed_or_wrong_instance_token_is_rejected() {
    for value in [
        None,
        Some("short-token"),
        Some("wrong-token-0123456789abcdefghijk"),
        Some("correct token 0123456789abcdefghij"),
    ] {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(
                INGEST_TOKEN_HEADER,
                HeaderValue::from_str(value)
                    .unwrap_or_else(|error| panic!("test header must be valid: {error}")),
            );
        }

        assert!(!verifier().authorizes(&headers), "must reject: {value:?}");
    }
}

#[test]
fn duplicate_instance_token_headers_are_rejected() {
    let mut headers = HeaderMap::new();
    headers.append(INGEST_TOKEN_HEADER, HeaderValue::from_static(TEST_TOKEN));
    headers.append(INGEST_TOKEN_HEADER, HeaderValue::from_static(TEST_TOKEN));

    assert!(!verifier().authorizes(&headers));
}

#[tokio::test]
async fn unauthorized_request_does_not_reach_clickhouse() {
    let mock = Mock::new();

    let response = app(&mock)
        .oneshot(post(Body::from(r#"{"table":"a","raw_data":1}"#), None))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn authorized_request_commits_the_insert_before_returning_no_content() {
    let mock = Mock::new();
    let recording = mock.add(handlers::record::<CapturedRawDataRow>());
    let body = serde_json::to_vec(&json!({
        "table": "synthetic_events",
        "raw_data": [1, {"nested": true}]
    }))
    .unwrap_or_else(|error| panic!("test JSON must serialize: {error}"));

    let response = app(&mock)
        .oneshot(post(Body::from(body), Some(TEST_TOKEN)))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));
    let rows: Vec<CapturedRawDataRow> = recording.collect().await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(rows.len(), 1);
    assert_ne!(rows[0].id, Uuid::nil());
    assert_eq!(rows[0].table_name, "synthetic_events");
    assert_eq!(rows[0].raw_data, r#"[1,{"nested":true}]"#);
    assert!(rows[0].received_at <= Utc::now());
}

#[tokio::test]
async fn invalid_table_name_is_a_client_error_without_an_insert() {
    let mock = Mock::new();

    let response = app(&mock)
        .oneshot(post(
            Body::from(r#"{"table":"   ","raw_data":null}"#),
            Some(TEST_TOKEN),
        ))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oversized_request_is_rejected_before_an_insert() {
    let mock = Mock::new();
    let body = format!(
        r#"{{"table":"synthetic_events","raw_data":"{}"}}"#,
        "x".repeat(MAX_REQUEST_BODY_BYTES)
    );

    let response = app(&mock)
        .oneshot(post(Body::from(body), Some(TEST_TOKEN)))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn saturated_gate_rejects_without_polling_the_body_or_clickhouse() {
    let mock = Mock::new();
    let state = state(&mock);
    let admission = IngestAdmission::new(&SecretString::from(TEST_TOKEN.to_owned()));
    let _permits: Vec<_> = (0..MAX_CONCURRENT_WRITES)
        .map(|_| {
            admission
                .write_slots
                .clone()
                .try_acquire_owned()
                .unwrap_or_else(|error| panic!("test must saturate the gate: {error}"))
        })
        .collect();
    let openapi = OpenApiRegistryImpl::new();
    let app = register_routes(Router::new(), &openapi, state, admission);

    let unauthorized = app
        .clone()
        .oneshot(post(
            unpollable_body(),
            Some("wrong-token-0123456789abcdefghijk"),
        ))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));
    let saturated = app
        .oneshot(post(unpollable_body(), Some(TEST_TOKEN)))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(saturated.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        saturated.headers().get(CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store"))
    );
}

#[tokio::test]
async fn missing_json_content_type_remains_unsupported_media_type() {
    let mock = Mock::new();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/raw-data")
        .header(INGEST_TOKEN_HEADER, TEST_TOKEN)
        .body(Body::from(r#"{"table":"synthetic_events","raw_data":1}"#))
        .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

    let response = app(&mock)
        .oneshot(request)
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));

    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn clickhouse_failure_returns_only_a_generic_error() {
    let mock = Mock::new();
    mock.add(handlers::failure(status::INTERNAL_SERVER_ERROR));

    let response = app(&mock)
        .oneshot(post(
            Body::from(r#"{"table":"synthetic_events","raw_data":1}"#),
            Some(TEST_TOKEN),
        ))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));
    let status = response.status();
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap_or_else(|error| panic!("response body must be readable: {error}"));
    let body = String::from_utf8_lossy(&body);

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.contains("ClickHouse"));
    assert!(!body.contains("database"));
}

#[tokio::test]
async fn openapi_documents_the_instance_token_and_timeout_response() {
    let mock = Mock::new();
    let openapi = OpenApiRegistryImpl::new();
    let state = state(&mock);
    let _ = register_routes(
        Router::new(),
        &openapi,
        state,
        IngestAdmission::new(&SecretString::from(TEST_TOKEN.to_owned())),
    );
    let document = openapi
        .build_openapi(&OpenApiInfo::default())
        .unwrap_or_else(|error| panic!("OpenAPI document must build: {error}"));
    let document = serde_json::to_value(document)
        .unwrap_or_else(|error| panic!("OpenAPI document must serialize: {error}"));
    let operation = &document["paths"]["/v1/raw-data"]["post"];
    let parameters = operation["parameters"]
        .as_array()
        .unwrap_or_else(|| panic!("raw-data parameters must be documented"));

    assert!(parameters.iter().any(|parameter| {
        parameter["name"] == "X-Insight-Token"
            && parameter["in"] == "header"
            && parameter["required"] == true
    }));
    assert!(operation["responses"].get("504").is_some());
}

#[test]
fn insert_timeout_is_reported_as_gateway_timeout() {
    let response = store_error(&StoreError::Timeout).into_response();

    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
}
