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
use crate::domain::datasets::Datasets as _;
use crate::domain::definition::Definitions;
use crate::domain::query::metric_query::MetricRunner;
use crate::store::definitions::memory::MemoryDefinitions;

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

/// A dataset that stands, ready to take records into `table`.
async fn a_ready_dataset(url: &str, table: &str) -> crate::api::Datasets {
    let rows = crate::store::datasets::memory::MemoryDatasets::at(chrono::Utc::now());
    let name = crate::domain::definition::DefinitionName::parse("synthetic_events")
        .unwrap_or_else(|error| panic!("the name parses: {error}"));
    let attempt = rows
        .take_create(
            &name,
            &serde_json::json!({ "title": "Events", "fields": [] }),
        )
        .await
        .unwrap_or_else(|error| panic!("the dataset is claimed: {error}"));
    for written in [
        crate::domain::datasets::Finish::Provisioned(table.to_owned()),
        crate::domain::datasets::Finish::Ready,
    ] {
        rows.finish(&name, &attempt.token, written)
            .await
            .unwrap_or_else(|error| panic!("the dataset is published: {error}"));
    }

    crate::api::Datasets::new(
        Arc::new(rows),
        crate::store::dataset_tables::DatasetTables::new(insight_clickhouse::Client::new(
            insight_clickhouse::Config::new(url, "insight_datasets"),
        )),
        "insight_datasets".to_owned(),
    )
}

fn state(mock: &Mock, datasets: crate::api::Datasets) -> Arc<AppState> {
    let url = mock.url();
    let definitions: Arc<dyn Definitions> = Arc::new(MemoryDefinitions::new());
    Arc::new(AppState::new(
        crate::api::Warehouse {
            catalog: crate::store::catalog::Catalog::new(
                insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                    "http://catalogue.invalid",
                    "insight",
                )),
                "insight".to_owned(),
            ),
            metrics: MetricRunner::new(
                insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "insight")),
                crate::domain::query::metric_query::People::new("identity"),
            ),
        },
        definitions.clone(),
        ChatClient::keyless(),
        crate::store::identity::IdentityClient::fixed(true),
        datasets,
    ))
}

/// The service with one dataset standing, which is what a sender writes into.
async fn app(mock: &Mock) -> Router {
    let datasets = a_ready_dataset(mock.url(), "ds_synthetic_events_1").await;

    with_datasets(mock, datasets)
}

fn with_datasets(mock: &Mock, datasets: crate::api::Datasets) -> Router {
    let openapi = OpenApiRegistryImpl::new();

    register_routes(
        Router::new(),
        &openapi,
        state(mock, datasets),
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
        .await
        .oneshot(post(Body::from(r#"{"dataset":"a","raw_data":1}"#), None))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn authorized_request_commits_the_insert_before_returning_no_content() {
    let mock = Mock::new();
    let recording = mock.add(handlers::record::<CapturedRawDataRow>());
    let body = serde_json::to_vec(&json!({
        "dataset": "synthetic_events",
        "raw_data": [1, {"nested": true}]
    }))
    .unwrap_or_else(|error| panic!("test JSON must serialize: {error}"));

    let response = app(&mock)
        .await
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
async fn a_name_no_dataset_could_have_is_not_found_and_reaches_no_table() {
    let mock = Mock::new();

    let response = app(&mock)
        .await
        .oneshot(post(
            Body::from(r#"{"dataset":"   ","raw_data":null}"#),
            Some(TEST_TOKEN),
        ))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn oversized_request_is_rejected_before_an_insert() {
    let mock = Mock::new();
    let body = format!(
        r#"{{"dataset":"synthetic_events","raw_data":"{}"}}"#,
        "x".repeat(MAX_REQUEST_BODY_BYTES)
    );

    let response = app(&mock)
        .await
        .oneshot(post(Body::from(body), Some(TEST_TOKEN)))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn saturated_gate_rejects_without_polling_the_body_or_clickhouse() {
    let mock = Mock::new();
    let state = state(
        &mock,
        a_ready_dataset(mock.url(), "ds_synthetic_events_1").await,
    );
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
        .body(Body::from(r#"{"dataset":"synthetic_events","raw_data":1}"#))
        .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

    let response = app(&mock)
        .await
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
        .await
        .oneshot(post(
            Body::from(r#"{"dataset":"synthetic_events","raw_data":1}"#),
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
    let state = state(
        &mock,
        a_ready_dataset(mock.url(), "ds_synthetic_events_1").await,
    );
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
fn a_store_that_did_not_answer_in_time_is_a_gateway_timeout() {
    let dataset =
        DefinitionName::parse("commits").unwrap_or_else(|error| panic!("the name parses: {error}"));

    let response = ingest_error(
        &dataset,
        IngestError::Table(crate::store::dataset_tables::DatasetTableError::Timeout),
    )
    .into_response();

    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
}

#[test]
fn a_record_named_for_a_dataset_that_is_not_ready_is_not_found() {
    let dataset =
        DefinitionName::parse("commits").unwrap_or_else(|error| panic!("the name parses: {error}"));

    let response = ingest_error(&dataset, IngestError::NotReady).into_response();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
