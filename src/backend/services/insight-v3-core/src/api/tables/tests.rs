use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use toolkit::api::OpenApiRegistryImpl;
use tower::ServiceExt as _;

use super::*;
use crate::api::AppState;
use crate::chat::ChatClient;
use crate::domain::query::metric_query::MetricRunner;
use crate::store::catalog::{Catalog, Column, Layer};
use crate::store::definitions::memory::MemoryDefinitions;

const BODY_LIMIT_BYTES: usize = 64 * 1024;

fn table(database: &str, table: &str, layer: Layer, columns: &[(&str, &str)]) -> TableSchema {
    TableSchema {
        database: database.to_owned(),
        table: table.to_owned(),
        layer,
        engine: "MergeTree".to_owned(),
        columns: columns
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).to_owned(),
                kind: (*kind).to_owned(),
            })
            .collect(),
    }
}

fn warehouse() -> Vec<TableSchema> {
    vec![
        table(
            "bronze_github",
            "issues",
            Layer::Bronze,
            &[("number", "Int64"), ("title", "String")],
        ),
        table("silver", "fct_commit", Layer::Silver, &[("sha", "String")]),
    ]
}

fn router(is_admin: bool) -> Router {
    let client = || {
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://clickhouse.invalid",
            "insight",
        ))
    };
    let state = Arc::new(AppState::new(
        MetricRunner::new(
            client(),
            crate::domain::query::metric_query::People::new("identity"),
        ),
        Arc::new(MemoryDefinitions::new()),
        ChatClient::keyless(),
        crate::store::identity::IdentityClient::fixed(is_admin),
        crate::api::Datasets::offline("http://offline.invalid"),
        Catalog::fixed(warehouse()),
    ));

    register_routes(Router::new(), &OpenApiRegistryImpl::new(), &state)
}

async fn get(router: &Router, path: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap_or_else(|error| panic!("test request must be valid: {error}"));
    let response = router
        .clone()
        .oneshot(request)
        .await
        .unwrap_or_else(|error| panic!("router must respond: {error}"));
    let status = response.status();
    let body: Bytes = to_bytes(response.into_body(), BODY_LIMIT_BYTES)
        .await
        .unwrap_or_else(|error| panic!("the body must be readable: {error}"));
    let value = serde_json::from_slice(&body).unwrap_or(Value::Null);

    (status, value)
}

#[tokio::test]
async fn every_table_is_listed_with_its_layer_and_nothing_more() {
    let (status, body) = get(&router(true), "/v1/tables").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "tables": [
                {"database": "bronze_github", "table": "issues", "layer": "bronze"},
                {"database": "silver", "table": "fct_commit", "layer": "silver"}
            ],
            "total": 2
        })
    );
}

#[tokio::test]
async fn a_listing_can_be_narrowed_to_one_database() {
    let (status, body) = get(&router(true), "/v1/tables?database=silver").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], json!(1));
    assert_eq!(body["tables"][0]["table"], json!("fct_commit"));
}

#[tokio::test]
async fn one_table_is_read_back_with_its_engine_and_columns_in_order() {
    let (status, body) = get(&router(true), "/v1/tables/bronze_github/issues").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "database": "bronze_github",
            "table": "issues",
            "layer": "bronze",
            "engine": "MergeTree",
            "columns": [
                {"name": "number", "type": "Int64"},
                {"name": "title", "type": "String"}
            ]
        })
    );
}

#[tokio::test]
async fn a_table_the_warehouse_does_not_hold_is_not_found() {
    let (status, body) = get(&router(true), "/v1/tables/bronze_github/commits").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body.to_string().contains("bronze_github.commits"),
        "the refusal names the table: {body}"
    );
}

#[tokio::test]
async fn the_catalogue_is_for_administrators_only() {
    for path in ["/v1/tables", "/v1/tables/silver/fct_commit"] {
        let (status, _) = get(&router(false), path).await;

        assert_eq!(status, StatusCode::FORBIDDEN, "at {path}");
    }
}
