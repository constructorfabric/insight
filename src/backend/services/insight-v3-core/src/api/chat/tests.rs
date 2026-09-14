use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use clickhouse::test::{Mock, handlers};
use serde::Deserialize;
use serde_json::json;
use toolkit::api::OpenApiRegistryImpl;
use tower::ServiceExt as _;

use super::*;
use crate::api::AppState;
use crate::chat::{ChatClient, Proposal};
use crate::definitions::Definitions;
use crate::definitions::memory::MemoryDefinitions;
use crate::metric_query::MetricRunner;
use crate::raw_data::RawDataStore;
use crate::tables::TableStore;

struct TestHarness {
    mock: Mock,
    router: Router,
    definitions: Arc<dyn Definitions>,
}

impl TestHarness {
    async fn new(chat: ChatClient) -> Self {
        Self::with_metrics(chat, None).await
    }

    /// A harness whose metric queries go somewhere that answers them, for the
    /// cases about what an answer does with the rows it got back.
    #[allow(clippy::unused_async)]
    async fn with_metrics(chat: ChatClient, metrics: Option<&str>) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let openapi = OpenApiRegistryImpl::new();
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
                insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                    metrics.unwrap_or(url),
                    "insight",
                )),
                crate::metric_query::People::new("identity"),
            ),
            chat,
            crate::identity::IdentityClient::fixed(true),
            crate::catalog::Catalog::new(
                insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                    "http://catalogue.invalid",
                    "insight",
                )),
                "insight".to_owned(),
            ),
        ));
        let router =
            crate::api::definitions::register_routes(Router::new(), &openapi, state.clone());
        let router = register_routes(router, &openapi, state);

        Self {
            mock,
            router,
            definitions,
        }
    }

    async fn put_json(&self, path: &str, body: serde_json::Value) -> TestResponse {
        let request = Request::builder()
            .method("PUT")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap_or_else(
                |error| panic!("test JSON must serialize: {error}"),
            )))
            .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .unwrap_or_else(|error| panic!("router must respond: {error}"));

        TestResponse::from_response(response).await
    }

    /// The only ClickHouse read a chat request makes before it answers: the
    /// list of ingest tables for the prompt. The definitions it reads come
    /// from the store, which needs no priming.
    fn queue_chat_context(&self) {
        self.mock.add(handlers::provide(Vec::<String>::new()));
    }

    /// What the store holds under `name`, for the cases about what a request
    /// wrote rather than what it answered.
    async fn stored(&self, kind: DefinitionKind, name: &str) -> serde_json::Value {
        let name = crate::definitions::DefinitionName::parse(name)
            .unwrap_or_else(|error| panic!("test name must parse: {error}"));

        self.definitions
            .get(kind, &name)
            .await
            .unwrap_or_else(|error| panic!("the store must answer: {error}"))
            .unwrap_or_else(|| panic!("nothing stored under {}", name.as_str()))
    }

    /// Posts `message` to `/v1/chat`.
    async fn post_chat(&self, message: &str) -> TestResponse {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/chat")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({ "message": message }))
                    .unwrap_or_else(|error| panic!("test JSON must serialize: {error}")),
            ))
            .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .unwrap_or_else(|error| panic!("router must respond: {error}"));

        TestResponse::from_response(response).await
    }
}

struct TestResponse {
    status: StatusCode,
    body: Bytes,
}

impl TestResponse {
    async fn from_response(response: axum::response::Response) -> Self {
        let status = response.status();
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap_or_else(|error| panic!("response body must be readable: {error}"));

        Self { status, body }
    }

    fn status(&self) -> StatusCode {
        self.status
    }

    #[allow(clippy::unused_async)]
    async fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("response body must be JSON: {error}"))
    }
}

#[derive(Debug, Default, Deserialize)]
#[allow(
    dead_code,
    reason = "mirrors the full response shape; not every field is asserted on"
)]
struct TestCreated {
    metric: Option<String>,
    widgets: Vec<String>,
    dashboard: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCreatedBody {
    #[serde(default)]
    #[allow(dead_code)]
    reply: String,
    #[serde(default)]
    created: TestCreated,
    #[serde(default)]
    updated: TestCreated,
}

/// An answer that promises rows over a table with none in it.
fn empty_answer_proposal() -> Proposal {
    Proposal::Answer {
        reply: "Here is who has committed the most overall.".to_owned(),
        query: Some(
            serde_json::from_value(json!({
                "database": "silver",
                "table": "fct_git_commit",
                "fields": [{ "column": "author_email", "type": "string", "as_name": "author" }],
                "group_by": ["author"],
                "filters": []
            }))
            .unwrap_or_else(|error| panic!("test query must parse: {error}")),
        ),
    }
}

fn single_existing_widget_proposal() -> Proposal {
    Proposal::Create {
        reply: "ok".to_owned(),
        // The metric arrives with the widget, as a chat request builds them.
        metric: Some((
            "m".to_owned(),
            json!({
                "table": "events",
                "fields": [{ "json": "day", "type": "string", "as_name": "day" }]
            }),
        )),
        widgets: vec![(
            "commits_table".to_owned(),
            json!({ "type": "table", "metric": "m", "columns": ["day"] }),
        )],
        dashboard: None,
    }
}

#[tokio::test]
async fn a_name_already_in_use_is_replaced_and_reported_as_updated() {
    let harness = TestHarness::new(ChatClient::scripted(single_existing_widget_proposal)).await;

    // The widget names this metric's columns, so it goes in first.
    harness
        .put_json(
            "/v1/metrics/was_here_first",
            json!({
                "table": "events",
                "fields": [{ "json": "day", "type": "string", "as_name": "day" }]
            }),
        )
        .await;
    let seeded = harness
        .put_json(
            "/v1/widgets/commits_table",
            json!({ "type": "table", "metric": "was_here_first", "columns": ["day"] }),
        )
        .await;
    assert_eq!(seeded.status(), StatusCode::NO_CONTENT);

    harness.queue_chat_context();

    let response = harness.post_chat("commits_table").await;
    assert_eq!(response.status(), StatusCode::OK);
    let created: ChatCreatedBody = serde_json::from_slice(&response.body)
        .unwrap_or_else(|error| panic!("response body must be JSON: {error}"));

    // Reusing a name is how the reader changes something by asking, so the
    // write goes through and the reply says which names it replaced.
    assert_eq!(created.updated.widgets, vec!["commits_table".to_owned()]);
    assert!(created.created.widgets.is_empty());

    let stored = harness
        .stored(DefinitionKind::Widget, "commits_table")
        .await;
    assert_eq!(stored["metric"], "m", "the new body must have replaced it");
}

/// A metric, two widgets over it and a dashboard holding both: everything one
/// proposal can carry, so the handler's whole store path is exercised.
fn whole_board_proposal() -> Proposal {
    Proposal::Create {
        reply: "here it is".to_owned(),
        metric: Some((
            "delivery_metric".to_owned(),
            json!({
                "table": "events",
                "fields": [
                    { "json": "day", "type": "string", "as_name": "day" },
                    { "json": "lines", "type": "int", "agg": "sum", "as_name": "lines" }
                ],
                "group_by": ["day"]
            }),
        )),
        widgets: vec![
            (
                "delivery".to_owned(),
                json!({
                    "type": "table",
                    "metric": "delivery_metric",
                    "columns": ["day", "lines"]
                }),
            ),
            (
                "delivery_line".to_owned(),
                json!({ "type": "line", "metric": "delivery_metric", "x": "day", "y": "lines" }),
            ),
        ],
        dashboard: Some((
            "delivery_dashboard".to_owned(),
            json!({ "title": "delivery", "widgets": ["delivery", "delivery_line"] }),
        )),
    }
}

#[tokio::test]
async fn a_proposal_stores_the_metric_its_widgets_and_the_dashboard_holding_them() {
    let harness = TestHarness::new(ChatClient::scripted(whole_board_proposal)).await;

    harness.queue_chat_context();

    let response = harness.post_chat("delivery report").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.json().await;

    assert_eq!(body["created"]["metric"], "delivery_metric");
    assert_eq!(
        body["created"]["widgets"],
        json!(["delivery", "delivery_line"])
    );
    assert_eq!(body["created"]["dashboard"], "delivery_dashboard");
    assert_eq!(
        body["updated"],
        json!({ "metric": null, "widgets": [], "dashboard": null })
    );
}

#[tokio::test]
async fn an_instance_with_no_key_refuses_the_ask_rather_than_inventing_a_reply() {
    let harness = TestHarness::new(ChatClient::keyless()).await;

    harness.queue_chat_context();

    let response = harness.post_chat("delivery report").await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

/// Serves one ClickHouse JSON result, and hands back where to reach it.
async fn upstream_returning(
    result: serde_json::Value,
) -> Result<(String, tokio::task::JoinHandle<Result<(), std::io::Error>>), Box<dyn std::error::Error>>
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let router = Router::new().route(
        "/",
        axum::routing::post(move || {
            let result = result.clone();
            async move { axum::Json(result) }
        }),
    );

    Ok((
        format!("http://{address}"),
        tokio::spawn(async move { axum::serve(listener, router).await }),
    ))
}

#[tokio::test]
async fn an_answer_whose_query_found_nothing_says_there_is_no_data()
-> Result<(), Box<dyn std::error::Error>> {
    // The stand holds empty tables beside full ones, and the reply is written
    // before the query runs - so the model's sentence claimed rows it never
    // saw, and the panel showed prose with nothing under it.
    let (metrics, server) = upstream_returning(json!({
        "meta": [{ "name": "author", "type": "String" }],
        "data": []
    }))
    .await?;
    let harness =
        TestHarness::with_metrics(ChatClient::scripted(empty_answer_proposal), Some(&metrics))
            .await;
    harness.queue_chat_context();

    let response = harness.post_chat("who has committed the most?").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.json().await;

    assert_eq!(
        body["reply"],
        "No data: that query returned no rows from silver.fct_git_commit."
    );
    server.abort();

    Ok(())
}

#[tokio::test]
async fn an_answer_with_rows_keeps_the_reply_it_came_with() -> Result<(), Box<dyn std::error::Error>>
{
    let (metrics, server) = upstream_returning(json!({
        "meta": [{ "name": "author", "type": "String" }],
        "data": [{ "author": "Liam Nguyen" }]
    }))
    .await?;
    let harness =
        TestHarness::with_metrics(ChatClient::scripted(empty_answer_proposal), Some(&metrics))
            .await;
    harness.queue_chat_context();

    let response = harness.post_chat("who has committed the most?").await;
    let body = response.json().await;

    assert_eq!(body["reply"], "Here is who has committed the most overall.");
    assert_eq!(body["result"]["rows"][0][0], "Liam Nguyen");
    server.abort();

    Ok(())
}
