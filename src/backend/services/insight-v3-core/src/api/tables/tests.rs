use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use clickhouse::test::{Mock, handlers};
use secrecy::SecretString;
use toolkit::api::{OpenApiInfo, OpenApiRegistryImpl};
use tower::ServiceExt as _;

use super::*;
use crate::api::AppState;
use crate::api::admission::{INGEST_TOKEN_HEADER, IngestAdmission};
use crate::chat::ChatClient;
use crate::definitions::Definitions;
use crate::definitions::memory::MemoryDefinitions;
use crate::metric_query::MetricRunner;
use crate::raw_data::RawDataStore;
use crate::tables::TableStore;

const TEST_TOKEN: &str = "correct-token-0123456789abcdefghi";

fn app(mock: &Mock, openapi: &OpenApiRegistryImpl) -> axum::Router {
    let url = mock.url();
    let definitions: Arc<dyn Definitions> = Arc::new(MemoryDefinitions::new());
    let state = Arc::new(AppState::new(
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
    ));

    register_routes(
        axum::Router::new(),
        openapi,
        state,
        IngestAdmission::new(&SecretString::from(TEST_TOKEN.to_owned())),
    )
}

fn put(table: &str, token: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method("PUT")
        .uri(format!("/v1/tables/{table}"));
    if let Some(token) = token {
        request = request.header(INGEST_TOKEN_HEADER, token);
    }

    request
        .body(Body::empty())
        .unwrap_or_else(|error| panic!("test request must be valid: {error}"))
}

#[tokio::test]
async fn create_table_is_bodyless_and_idempotent() {
    let mock = Mock::new();
    mock.add(handlers::record_ddl());
    let openapi = OpenApiRegistryImpl::new();

    let response = app(&mock, &openapi)
        .oneshot(put("events_2026", Some(TEST_TOKEN)))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn invalid_name_and_missing_token_do_not_reach_clickhouse() {
    let mock = Mock::new();
    let openapi = OpenApiRegistryImpl::new();
    let router = app(&mock, &openapi);

    let invalid = router
        .clone()
        .oneshot(put("events.daily", Some(TEST_TOKEN)))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));
    let unauthorized = router
        .oneshot(put("events", None))
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));

    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn openapi_documents_bodyless_table_creation_and_instance_token() {
    let mock = Mock::new();
    let openapi = OpenApiRegistryImpl::new();
    let _ = app(&mock, &openapi);
    let document = openapi
        .build_openapi(&OpenApiInfo::default())
        .unwrap_or_else(|error| panic!("OpenAPI document must build: {error}"));
    let document = serde_json::to_value(document)
        .unwrap_or_else(|error| panic!("OpenAPI document must serialize: {error}"));
    let operation = &document["paths"]["/v1/tables/{table}"]["put"];

    assert!(operation.get("requestBody").is_none());
    assert!(
        operation["parameters"]
            .as_array()
            .is_some_and(|parameters| {
                parameters.iter().any(|parameter| {
                    parameter["name"] == "X-Insight-Token"
                        && parameter["in"] == "header"
                        && parameter["required"] == true
                })
            })
    );
}
