use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Router};
use chrono::{DateTime, Utc};
use clickhouse::test::{Handler, handlers};
use sea_orm::{DatabaseBackend, MockDatabase};
use serde::Serialize;
use serde_json::json;
use tower::ServiceExt;

use super::support::*;
use crate::infra::db::entities::connector_schedules::Model;

#[derive(Debug, Serialize, clickhouse::Row)]
struct PassCompletion {
    tick_id: &'static str,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    sealed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct Interval {
    interval_ms: u64,
    gaps: u64,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct ConfiguredInstance {
    connector: &'static str,
    tenant_id: &'static str,
    source_id: &'static str,
    schedules: Vec<String>,
    schedule_timezone: &'static str,
    schedule_origin: &'static str,
    schedule_suspended: bool,
}

fn rows<T: Serialize + clickhouse::Row>(rows: impl IntoIterator<Item = T>) -> Response {
    let (handler, _) = handlers::provide(rows).make();
    handler(Request::new(Bytes::new())).map(Body::from)
}

async fn sync_history(query: Bytes) -> Response {
    let query = String::from_utf8_lossy(&query);
    if query.contains("ts AS sealed_at") {
        return rows([PassCompletion {
            tick_id: "stand-scratch-schedule",
            sealed_at: DateTime::UNIX_EPOCH,
        }]);
    }
    if query.contains("AS interval_ms") {
        return rows([Interval {
            interval_ms: 0,
            gaps: 0,
        }]);
    }
    if query.contains("SELECT DISTINCT connector") {
        return rows([ConfiguredInstance {
            connector: "example-tracker",
            tenant_id: "example-tenant",
            source_id: "main",
            schedules: vec!["0 3 * * *".to_owned()],
            schedule_timezone: "UTC",
            schedule_origin: "default",
            schedule_suspended: false,
        }]);
    }
    assert!(query.contains("argMax"), "{query}");
    Response::new(Body::empty())
}

async fn health_requests(through_router: bool) -> R {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!(
        "http://mocked.clickhouse.test:{}",
        listener.local_addr()?.port()
    );
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/", axum::routing::post(sync_history)),
        )
        .await
    });
    let server = server.abort_handle();
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([vec![
            crate::domain::connector_schedules::test_support::snapshot_row(
                Some(&Model {
                    connector: "example-tracker".to_owned(),
                    source_id: "main".to_owned(),
                    crons: json!(["0 6 * * *"]),
                    revision: 1,
                    updated_by: CALLER,
                    updated_at: DateTime::UNIX_EPOCH,
                }),
                None,
            ),
        ]])
        .into_connection();
    let h = harness(db, &url, context("user", &[])?, true).await?;
    let response = if through_router {
        h.app
            .clone()
            .oneshot(request("GET", "/v1/connector-health", None, &json!(null))?)
            .await?
    } else {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer synthetic-token"),
        );
        crate::api::connector_health::get_connector_health(Extension(Arc::clone(&h.state)), headers)
            .await?
            .into_response()
    };
    assert_eq!(response.status(), StatusCode::OK);
    let response = body(response).await?;
    let state = &response["connectors"][0]["schedule_request"];
    assert_eq!(state["crons"], json!(["0 6 * * *"]));
    assert_eq!(state["status"], "pending");
    server.abort();
    Ok(())
}

#[tokio::test]
async fn health_handler_enriches_schedule_requests() -> R {
    health_requests(false).await
}

#[tokio::test]
async fn health_route_enriches_schedule_requests() -> R {
    health_requests(true).await
}
