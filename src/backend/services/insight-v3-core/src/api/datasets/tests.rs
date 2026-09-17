use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use clickhouse::test::{Mock, handlers};
use serde::Serialize;
use serde_json::json;
use toolkit::api::OpenApiRegistryImpl;
use tower::ServiceExt as _;

use super::*;
use crate::api::AppState;
use crate::chat::ChatClient;
use crate::domain::query::metric_query::MetricRunner;
use crate::store::definitions::memory::MemoryDefinitions;

const BODY_LIMIT_BYTES: usize = 64 * 1024;

/// What `system.tables` answers about a name nothing holds.
#[derive(Debug, Serialize, clickhouse::Row)]
struct NoTable {
    sorting_key: String,
}

struct TestHarness {
    mock: Mock,
    router: Router,
}

impl TestHarness {
    fn new() -> Self {
        Self::with_caller(true)
    }

    /// A caller who does or does not hold the admin role.
    fn with_caller(is_admin: bool) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let openapi = OpenApiRegistryImpl::new();
        let url = mock.url();
        let datasets = crate::api::Datasets::new(
            Arc::new(crate::store::datasets::memory::MemoryDatasets::at(
                chrono::Utc::now(),
            )),
            crate::store::dataset_tables::DatasetTables::new(insight_clickhouse::Client::new(
                insight_clickhouse::Config::new(url, "insight_datasets"),
            )),
            "insight_datasets".to_owned(),
        );
        let state = Arc::new(AppState::new(
            crate::api::Warehouse {
                catalog: crate::store::catalog::Catalog::new(
                    insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                        url, "insight",
                    )),
                    "insight".to_owned(),
                ),
                metrics: MetricRunner::new(
                    insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                        url, "insight",
                    )),
                    crate::domain::query::metric_query::People::new("identity"),
                ),
            },
            Arc::new(MemoryDefinitions::new()),
            ChatClient::keyless(),
            crate::store::identity::IdentityClient::fixed(is_admin),
            datasets,
        ));
        let router = register_routes(Router::new(), &openapi, &state);

        Self { mock, router }
    }

    /// The datasets database answers that nothing holds the name, and takes
    /// whatever statement follows.
    fn a_free_name(&self) {
        self.mock.add(handlers::provide(Vec::<NoTable>::new()));
        self.mock.add(handlers::record_ddl());
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, Bytes) {
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .unwrap_or_else(|error| panic!("router must respond: {error}"));
        let status = response.status();
        let body = to_bytes(response.into_body(), BODY_LIMIT_BYTES)
            .await
            .unwrap_or_else(|error| panic!("the body must be readable: {error}"));

        (status, body)
    }

    async fn put(&self, name: &str, body: serde_json::Value) -> (StatusCode, Bytes) {
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/v1/datasets/{name}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap_or_else(
                |error| panic!("test JSON must serialize: {error}"),
            )))
            .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        self.send(request).await
    }

    async fn get(&self, path: &str) -> (StatusCode, Bytes) {
        let request = Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
            .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        self.send(request).await
    }

    async fn delete(&self, name: &str) -> (StatusCode, Bytes) {
        let request = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/datasets/{name}"))
            .body(Body::empty())
            .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        self.send(request).await
    }
}

fn declaration() -> serde_json::Value {
    json!({
        "title": "Commits",
        "fields": [
            { "name": "day", "path": "day", "type": "datetime", "default_clock": true },
            { "name": "lines", "path": "lines", "type": "int" }
        ]
    })
}

fn read(body: &Bytes) -> serde_json::Value {
    serde_json::from_slice(body).unwrap_or_else(|error| panic!("the body is JSON: {error}"))
}

#[tokio::test]
async fn a_declared_dataset_is_answered_back_and_can_be_read_again() {
    let harness = TestHarness::new();
    harness.a_free_name();

    let (declared, body) = harness.put("commits", declaration()).await;

    assert_eq!(declared, StatusCode::OK);
    assert_eq!(read(&body)["declaration"], declaration());

    let (found, body) = harness.get("/v1/datasets/commits").await;
    assert_eq!(found, StatusCode::OK);
    assert_eq!(read(&body)["name"], "commits");
    assert_eq!(read(&body)["declaration"], declaration());
}

#[tokio::test]
async fn a_declaration_wrong_in_several_places_is_answered_with_all_of_them() {
    let harness = TestHarness::new();

    let (refused, body) = harness
        .put(
            "commits",
            json!({
                "title": "Commits",
                "fields": [
                    { "name": "day", "path": "day", "type": "datetime" },
                    { "name": "day", "path": "other", "type": "int" }
                ],
                "row_identity": ["nowhere"]
            }),
        )
        .await;

    assert_eq!(refused, StatusCode::BAD_REQUEST);
    let said = String::from_utf8_lossy(&body);
    assert!(said.contains("fields[1].name"), "{said}");
    assert!(said.contains("row_identity[0]"), "{said}");
}

#[tokio::test]
async fn a_dataset_nobody_declared_is_not_found() {
    let harness = TestHarness::new();

    let (answered, _) = harness.get("/v1/datasets/commits").await;

    assert_eq!(answered, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn declaring_a_dataset_needs_the_admin_role() {
    let harness = TestHarness::with_caller(false);

    let (refused, _) = harness.put("commits", declaration()).await;
    let (removal, _) = harness.delete("commits").await;

    assert_eq!(refused, StatusCode::FORBIDDEN);
    assert_eq!(removal, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn every_dataset_is_listed_by_name() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;

    let (listed, body) = harness.get("/v1/datasets").await;

    assert_eq!(listed, StatusCode::OK);
    assert_eq!(read(&body)["names"], json!(["commits"]));
}

#[tokio::test]
async fn a_removed_dataset_leaves_the_catalogue() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;
    harness.mock.add(handlers::provide(vec![NoTable {
        sorting_key: "table_name, received_at, id".to_owned(),
    }]));
    harness.mock.add(handlers::record_ddl());

    let (removed, _) = harness.delete("commits").await;

    assert_eq!(removed, StatusCode::NO_CONTENT);
    let (listed, body) = harness.get("/v1/datasets").await;
    assert_eq!(listed, StatusCode::OK);
    assert_eq!(read(&body)["names"], json!([]));
}

#[tokio::test]
async fn a_name_outside_the_charset_is_refused_before_anything_is_read() {
    let harness = TestHarness::new();

    let (refused, _) = harness.put("drop%20table", declaration()).await;

    assert_eq!(refused, StatusCode::BAD_REQUEST);
}
