use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use clickhouse::test::{Mock, handlers};
use serde_json::json;
use toolkit::api::OpenApiRegistryImpl;
use tower::ServiceExt as _;

use super::*;
use crate::api::AppState;
use crate::chat::{ChatClient, Proposal, Proposed};
use crate::domain::datasets::Datasets as _;
use crate::domain::definition::{Definitions as _, Lookup as _};
use crate::domain::query::metric_query::MetricRunner;
use crate::store::definitions::memory::MemoryDefinitions;

struct TestHarness {
    mock: Mock,
    router: Router,
    definitions: Arc<MemoryDefinitions>,
}

fn proposed(name: &str, body: serde_json::Value) -> Proposed {
    Proposed {
        id: None,
        name: name.to_owned(),
        body,
    }
}

fn replacing(id: &str, name: &str, body: serde_json::Value) -> Proposed {
    Proposed {
        id: Some(id.to_owned()),
        ..proposed(name, body)
    }
}

/// The one dataset the assistant reads in these cases.
fn a_ready_dataset(url: &str) -> crate::api::Datasets {
    let rows = crate::store::datasets::memory::MemoryDatasets::at(chrono::Utc::now());
    let name = crate::domain::definition::DefinitionId::parse("commits")
        .unwrap_or_else(|error| panic!("the name parses: {error}"));
    let declaration = json!({
        "title": "Commits",
        "fields": [
            { "name": "author_email", "path": "author_email", "type": "string" },
            { "name": "day", "path": "day", "type": "string" },
            { "name": "lines", "path": "lines", "type": "int" }
        ]
    });

    futures::executor::block_on(async {
        let attempt = rows
            .take_create(&name, &declaration)
            .await
            .unwrap_or_else(|error| panic!("the dataset is claimed: {error}"))
            .attempt();
        for written in [
            crate::domain::datasets::Finish::Provisioned("ds_commits_1".to_owned()),
            crate::domain::datasets::Finish::Ready,
        ] {
            rows.finish(&name, &attempt.token, written)
                .await
                .unwrap_or_else(|error| panic!("the dataset is published: {error}"));
        }
    });

    crate::api::Datasets::new(
        std::sync::Arc::new(rows),
        crate::store::dataset_tables::DatasetTables::new(insight_clickhouse::Client::new(
            insight_clickhouse::Config::new(url, "insight_datasets"),
        )),
        "insight_datasets".to_owned(),
        50,
    )
}

impl TestHarness {
    async fn new(chat: ChatClient) -> Self {
        Self::with_metrics(chat, None).await
    }

    /// A harness whose metric queries go somewhere that answers them, for the
    /// cases about what an answer does with the rows it got back.
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "the harness mirrors the async one"
    )]
    async fn with_metrics(chat: ChatClient, metrics: Option<&str>) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let openapi = OpenApiRegistryImpl::new();
        let url = mock.url();
        let definitions = Arc::new(MemoryDefinitions::new());
        let state = Arc::new(AppState::new(
            MetricRunner::new(
                insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                    metrics.unwrap_or(url),
                    "insight",
                )),
                crate::domain::query::metric_query::People::new("identity"),
            ),
            definitions.clone(),
            chat,
            crate::store::identity::IdentityClient::fixed(true),
            a_ready_dataset(url),
            crate::store::catalog::Catalog::fixed(Vec::new()),
        ));
        let router = crate::api::definitions::register_routes(Router::new(), &openapi, &state);
        let router = register_routes(router, &openapi, state);

        Self {
            mock,
            router,
            definitions,
        }
    }

    /// The only ClickHouse read a chat request makes before it answers: the
    /// list of ingest tables for the prompt. The definitions it reads come
    /// from the store, which needs no priming.
    fn queue_chat_context(&self) {
        self.mock.add(handlers::provide(Vec::<String>::new()));
    }

    /// Whether the store holds any definition of this kind.
    async fn holds_any(&self, kind: DefinitionKind) -> bool {
        !self
            .definitions
            .list(kind)
            .await
            .unwrap_or_else(|error| panic!("the store must answer: {error}"))
            .is_empty()
    }

    /// What the store holds under `id`, for the cases about what a request
    /// wrote rather than what it answered.
    async fn stored(&self, kind: DefinitionKind, id: &str) -> serde_json::Value {
        let parsed = crate::domain::definition::DefinitionId::parse(id)
            .unwrap_or_else(|error| panic!("the id must parse: {error}"));

        self.definitions
            .get(kind, &parsed)
            .await
            .unwrap_or_else(|error| panic!("the store must answer: {error}"))
            .unwrap_or_else(|| panic!("nothing stored under {id}"))
    }

    async fn seed(&self, kind: DefinitionKind, id: &str, body: serde_json::Value) {
        let parsed = crate::domain::definition::DefinitionId::parse(id)
            .unwrap_or_else(|error| panic!("the id must parse: {error}"));

        self.definitions
            .seed(kind, &parsed, &body)
            .await
            .unwrap_or_else(|error| panic!("the store must accept it: {error}"));
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

    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "the harness mirrors the async one"
    )]
    async fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("response body must be JSON: {error}"))
    }
}

/// The id the response reports for a written item, as a string.
fn reported_id(written: &serde_json::Value) -> String {
    written["id"]
        .as_str()
        .unwrap_or_else(|| panic!("an id is reported: {written}"))
        .to_owned()
}

/// An answer that promises rows over a table with none in it.
fn empty_answer_proposal() -> Proposal {
    Proposal::Answer {
        reply: "Here is who has committed the most overall.".to_owned(),
        query: Some(
            serde_json::from_value(json!({
                                "dataset": "commits",
                "fields": [{ "field": "author_email", "type": "string", "as_name": "author" }],
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
        metric: Some(proposed(
            "m",
            json!({
                "dataset": "commits",
                "fields": [{ "field": "day", "type": "string", "as_name": "day" }]
            }),
        )),
        widgets: vec![replacing(
            "commits_table",
            "Commits table",
            json!({ "type": "table", "metric": "m", "columns": ["day"] }),
        )],
        dashboard: None,
    }
}

#[tokio::test]
async fn an_item_carrying_a_stored_id_replaces_it_and_is_reported_as_updated() {
    let harness = TestHarness::new(ChatClient::scripted(single_existing_widget_proposal)).await;
    let first = json!({
        "dataset": "commits",
        "fields": [{ "field": "day", "type": "string", "as_name": "day" }]
    });
    harness
        .seed(DefinitionKind::Metric, "was_here_first", first)
        .await;
    harness
        .seed(
            DefinitionKind::Widget,
            "commits_table",
            json!({ "type": "table", "metric": "was_here_first", "columns": ["day"] }),
        )
        .await;
    harness.queue_chat_context();

    let response = harness.post_chat("commits_table").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.json().await;

    assert_eq!(
        body["updated"]["widgets"],
        json!([{ "id": "commits_table", "name": "Commits table" }])
    );
    assert_eq!(body["created"]["widgets"], json!([]));
    let metric = reported_id(&body["created"]["metric"]);
    assert_ne!(metric, "m", "a created item is given an id of its own");
    let stored = harness
        .stored(DefinitionKind::Widget, "commits_table")
        .await;
    assert_eq!(
        stored["metric"],
        json!(metric),
        "the widget must draw the metric the same proposal created"
    );
}

fn a_widget_replacing_an_id_nobody_holds() -> Proposal {
    Proposal::Create {
        reply: "ok".to_owned(),
        metric: Some(proposed(
            "m",
            json!({
                "dataset": "commits",
                "fields": [{ "field": "day", "type": "string", "as_name": "day" }]
            }),
        )),
        widgets: vec![replacing(
            "nowhere",
            "Chart",
            json!({ "type": "table", "metric": "m", "columns": ["day"] }),
        )],
        dashboard: None,
    }
}

#[tokio::test]
async fn an_item_replacing_an_id_nobody_holds_stores_none_of_the_proposal() {
    let harness =
        TestHarness::new(ChatClient::scripted(a_widget_replacing_an_id_nobody_holds)).await;
    harness.queue_chat_context();

    let refused = harness.post_chat("change the chart").await;

    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    assert!(!harness.holds_any(DefinitionKind::Metric).await);
    assert!(!harness.holds_any(DefinitionKind::Widget).await);
}

/// A metric, two widgets over it and a dashboard holding both: everything one
/// proposal can carry, so the handler's whole store path is exercised.
fn whole_board_proposal() -> Proposal {
    Proposal::Create {
        reply: "here it is".to_owned(),
        metric: Some(proposed(
            "Delivery metric",
            json!({
                "dataset": "commits",
                "fields": [
                    { "field": "day", "type": "string", "as_name": "day" },
                    { "field": "lines", "type": "int", "agg": "sum", "as_name": "lines" }
                ],
                "group_by": ["day"]
            }),
        )),
        widgets: vec![
            proposed(
                "Delivery",
                json!({
                    "type": "table",
                    "metric": "Delivery metric",
                    "columns": ["day", "lines"]
                }),
            ),
            proposed(
                "Delivery line",
                json!({ "type": "line", "metric": "Delivery metric", "x": "day", "y": "lines" }),
            ),
        ],
        dashboard: Some(proposed(
            "Delivery board",
            json!({ "title": "delivery", "items": [{ "widget": "Delivery" }, { "heading": "Trend" }, { "widget": "Delivery line" }] }),
        )),
    }
}

#[tokio::test]
async fn a_proposal_lands_with_ids_and_its_items_point_at_each_other_by_them() {
    let harness = TestHarness::new(ChatClient::scripted(whole_board_proposal)).await;

    harness.queue_chat_context();

    let response = harness.post_chat("delivery report").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.json().await;

    let created = &body["created"];
    assert_eq!(created["metric"]["name"], "Delivery metric");
    assert_eq!(created["dashboard"]["name"], "Delivery board");
    let metric = reported_id(&created["metric"]);
    let widgets: Vec<String> = created["widgets"]
        .as_array()
        .unwrap_or_else(|| panic!("widgets are listed: {body}"))
        .iter()
        .map(reported_id)
        .collect();
    let dashboard = reported_id(&created["dashboard"]);
    assert_eq!(widgets.len(), 2, "{body}");
    assert_eq!(
        body["updated"],
        json!({ "metric": null, "widgets": [], "dashboard": null })
    );

    for widget in &widgets {
        let stored = harness.stored(DefinitionKind::Widget, widget).await;
        assert_eq!(stored["metric"], json!(metric), "widget {widget}: {stored}");
    }
    let board = harness.stored(DefinitionKind::Dashboard, &dashboard).await;
    assert_eq!(
        board["items"],
        json!([{ "widget": widgets[0] }, { "heading": "Trend" }, { "widget": widgets[1] }])
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
        "No data: that query returned no rows from commits."
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

fn a_widget_naming_a_column_the_metric_lacks() -> Proposal {
    Proposal::Create {
        reply: "ok".to_owned(),
        metric: Some(proposed(
            "m",
            json!({
                "dataset": "commits",
                "fields": [{ "field": "day", "type": "string", "as_name": "day" }]
            }),
        )),
        widgets: vec![proposed(
            "chart",
            json!({ "type": "table", "metric": "m", "columns": ["nowhere"] }),
        )],
        dashboard: None,
    }
}

fn a_metric_that_cannot_be_read_as_a_query() -> Proposal {
    Proposal::Create {
        reply: "ok".to_owned(),
        metric: Some(proposed("m", json!({ "dataset": "commits" }))),
        widgets: Vec::new(),
        dashboard: None,
    }
}

#[tokio::test]
async fn a_proposal_with_one_refused_body_stores_none_of_it() {
    let harness = TestHarness::new(ChatClient::scripted(
        a_widget_naming_a_column_the_metric_lacks,
    ))
    .await;
    harness.queue_chat_context();

    let refused = harness.post_chat("build me a board").await;

    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert!(
        !harness.holds_any(DefinitionKind::Metric).await,
        "the metric of a refused batch must not be stored"
    );
    assert!(!harness.holds_any(DefinitionKind::Widget).await);
}

#[tokio::test]
async fn a_proposed_metric_that_cannot_be_read_as_a_query_is_refused_before_it_is_stored() {
    let harness = TestHarness::new(ChatClient::scripted(
        a_metric_that_cannot_be_read_as_a_query,
    ))
    .await;
    harness.queue_chat_context();

    let refused = harness.post_chat("build me a metric").await;

    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert!(!harness.holds_any(DefinitionKind::Metric).await);
}
