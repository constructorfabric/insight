use std::sync::Arc;

use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use clickhouse::test::Mock;
use serde_json::json;
use tower::ServiceExt as _;

use super::*;
use crate::api::AppState;
use crate::chat::ChatClient;
use crate::domain::definition::Definitions;
use crate::domain::query::metric_query::MetricRunner;
use crate::store::definitions::memory::MemoryDefinitions;

type R = Result<(), Box<dyn std::error::Error>>;

struct TestHarness {
    /// Held, not read: the stores this harness does not exercise are built
    /// against its address, so it has to outlive them.
    _clickhouse: Mock,
    router: Router,
    definitions: Arc<dyn Definitions>,
}

/// The one dataset every run here reads: a record with a moment, a day and a
/// number in it.
fn a_ready_dataset(url: &str) -> crate::api::Datasets {
    crate::api::Datasets::holding(
        url,
        &[(
            "commits",
            json!({
            "title": "Commits",
            "fields": [
                { "name": "occurred_at", "path": "occurred_at", "type": "datetime" },
                { "name": "day", "path": "day", "type": "string" },
                    { "name": "lines", "path": "lines", "type": "int" }
                ]
            }),
        )],
    )
}

impl TestHarness {
    #[allow(clippy::unused_async)]
    async fn new(metrics_url: &str) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let openapi = toolkit::api::OpenApiRegistryImpl::new();
        let metrics_client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            metrics_url,
            "insight",
        ));
        let definitions: Arc<dyn Definitions> = Arc::new(MemoryDefinitions::new());
        let datasets = a_ready_dataset(metrics_url);
        let state = Arc::new(AppState::new(
            MetricRunner::new(
                metrics_client,
                crate::domain::query::metric_query::People::new("identity"),
            ),
            definitions.clone(),
            ChatClient::keyless(),
            crate::store::identity::IdentityClient::fixed(true),
            datasets,
        ));
        let router = register_routes(Router::new(), &openapi, state);

        Self {
            _clickhouse: mock,
            router,
            definitions,
        }
    }

    async fn run(&self, name: &str, stored: Option<serde_json::Value>) -> TestResponse {
        self.ask(name, stored, None).await
    }

    async fn ask(
        &self,
        name: &str,
        stored: Option<serde_json::Value>,
        asked: Option<serde_json::Value>,
    ) -> TestResponse {
        if let Some(body) = stored {
            let parsed = crate::domain::definition::DefinitionName::parse(name)
                .unwrap_or_else(|error| panic!("test name must parse: {error}"));
            self.definitions
                .put(
                    crate::domain::definition::DefinitionKind::Metric,
                    &parsed,
                    &body,
                )
                .await
                .unwrap_or_else(|error| panic!("the store must accept it: {error}"));
        }

        let builder = Request::builder()
            .method("POST")
            .uri(format!("/v1/metrics/{name}/run"));
        let request = match asked {
            Some(asked) => builder
                .header("content-type", "application/json")
                .body(Body::from(asked.to_string())),
            None => builder.body(Body::empty()),
        }
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

type Seen = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

async fn clocked_upstream() -> (String, tokio::task::JoinHandle<()>) {
    let (address, server, _) = recording_upstream().await;

    (address, server)
}

async fn recording_upstream() -> (String, tokio::task::JoinHandle<()>, Seen) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("the upstream must bind: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("the upstream must have an address: {error}"));

    let seen: Seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = seen.clone();
    let upstream = Router::new().route(
        "/",
        post(move |sql: String| {
            let recorded = recorded.clone();
            async move {
                if sql.contains("countIf(isNull(") {
                    return Json(json!({
                        "meta": [],
                        "data": [{"newest": "1757516400000", "undated": "4"}]
                    }));
                }

                if let Ok(mut recorded) = recorded.lock() {
                    recorded.push(sql);
                }

                Json(json!({
                    "meta": [{"name": "bucket", "type": "DateTime"}, {"name": "total", "type": "UInt64"}],
                    "data": [{"bucket": "2026-09-10 00:00:00", "total": "2"}]
                }))
            }
        }),
    );
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, upstream).await;
    });

    (format!("http://{address}"), server, seen)
}

fn only_read(seen: &Seen) -> String {
    let recorded = seen
        .lock()
        .unwrap_or_else(|error| panic!("the recording must be readable: {error}"));
    let [sql] = recorded.as_slice() else {
        panic!(
            "exactly one metric read was expected, saw {}",
            recorded.len()
        );
    };

    sql.clone()
}

fn midnights(day: chrono::NaiveDate) -> [i64; 2] {
    let midnight = |date: chrono::NaiveDate| {
        date.and_hms_opt(0, 0, 0)
            .unwrap_or_else(|| panic!("midnight exists on {date}"))
            .and_utc()
            .timestamp_millis()
    };
    let yesterday = day
        .pred_opt()
        .unwrap_or_else(|| panic!("{day} has a day before it"));

    [midnight(yesterday), midnight(day)]
}

fn clocked_metric() -> serde_json::Value {
    json!({
        "dataset": "commits",
        "time": { "field": "occurred_at" },
        "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
    })
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

#[tokio::test]
async fn a_stored_metric_runs_and_returns_columns_in_field_order() -> R {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let upstream = Router::new().route(
        "/",
        post(|| async {
            Json(json!({
                "meta": [{"name": "day", "type": "String"}, {"name": "lines", "type": "UInt64"}],
                "data": [{"day": "2026-09-01", "lines": "3"}]
            }))
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await });

    let harness = TestHarness::new(&format!("http://{address}")).await;
    let metric = json!({
        "dataset": "commits",
        "fields": [
            { "field": "day", "type": "string", "as_name": "day" },
            { "field": "lines", "type": "int", "agg": "sum", "as_name": "lines" }
        ],
        "group_by": ["day"],
        "filters": [],
        "limit": 100
    });

    let response = harness.run("commits_per_day", Some(metric)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json().await,
        json!({
            "columns": ["day", "lines"],
            "rows": [["2026-09-01", 3]],
            "percents": []
        })
    );

    server.abort();
    Ok(())
}

#[tokio::test]
async fn a_missing_metric_is_not_found() -> R {
    let harness = TestHarness::new("http://127.0.0.1:1").await;

    let response = harness.run("nope", None).await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn an_uncompilable_metric_is_a_bad_request() -> R {
    let harness = TestHarness::new("http://127.0.0.1:1").await;
    let metric = json!({ "dataset": "commits", "fields": [], "group_by": [], "filters": [] });

    let response = harness.run("empty", Some(metric)).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn a_name_outside_the_charset_is_rejected() -> R {
    let harness = TestHarness::new("http://127.0.0.1:1").await;

    let request = Request::builder()
        .method("POST")
        .uri("/v1/metrics/drop%20table/run")
        .body(Body::empty())?;
    let response = harness.router.clone().oneshot(request).await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn a_ranged_run_buckets_its_rows_and_says_how_many_had_no_clock() -> R {
    let (address, server) = clocked_upstream().await;
    let harness = TestHarness::new(&address).await;

    let response = harness
        .ask(
            "opened",
            Some(clocked_metric()),
            Some(json!({"range": "P7D"})),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json().await,
        json!({
            "columns": ["bucket", "total"],
            "rows": [["2026-09-10 00:00:00", 2]],
            "percents": [],
            "undated": 4,
            "clock": { "field": "occurred_at", "from": "metric" }
        })
    );

    server.abort();
    Ok(())
}

#[tokio::test]
async fn a_run_with_no_body_says_nothing_about_rows_without_a_clock() -> R {
    let (address, server) = clocked_upstream().await;
    let harness = TestHarness::new(&address).await;

    let response = harness.run("opened", Some(clocked_metric())).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.json().await.get("undated"), None);

    server.abort();
    Ok(())
}

#[tokio::test]
async fn a_range_the_server_does_not_know_names_the_range_field() -> R {
    let harness = TestHarness::new("http://127.0.0.1:1").await;

    let response = harness
        .ask(
            "opened",
            Some(clocked_metric()),
            Some(json!({"range": "P14D"})),
        )
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response.json().await.to_string().contains("range"),
        "the refusal must name the field"
    );

    Ok(())
}

#[tokio::test]
async fn a_zone_the_server_does_not_know_names_the_tz_field() -> R {
    let harness = TestHarness::new("http://127.0.0.1:1").await;

    let response = harness
        .ask(
            "opened",
            Some(clocked_metric()),
            Some(json!({"range": "P7D", "tz": "Mars/Olympus"})),
        )
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response.json().await.to_string().contains("tz"),
        "the refusal must name the field"
    );

    Ok(())
}

#[tokio::test]
async fn a_body_whose_fields_are_the_wrong_shape_is_a_bad_request() -> R {
    let harness = TestHarness::new("http://127.0.0.1:1").await;

    let response = harness
        .ask("opened", Some(clocked_metric()), Some(json!({"range": 7})))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn a_range_asked_of_a_metric_with_no_clock_is_a_bad_request() -> R {
    let harness = TestHarness::new("http://127.0.0.1:1").await;
    let clockless = json!({
        "dataset": "commits",
        "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
    });

    let response = harness
        .ask("all_time", Some(clockless), Some(json!({"range": "P7D"})))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn yesterday_is_the_day_before_now_however_old_the_newest_row_is() -> R {
    let (address, server, seen) = recording_upstream().await;
    let harness = TestHarness::new(&address).await;

    let before = chrono::Utc::now().date_naive();
    let response = harness
        .ask(
            "opened",
            Some(clocked_metric()),
            Some(json!({"range": "PDC"})),
        )
        .await;
    let after = chrono::Utc::now().date_naive();
    assert_eq!(response.status(), StatusCode::OK);

    let sql = only_read(&seen);
    assert!(
        [before, after].iter().any(|day| midnights(*day)
            .iter()
            .all(|edge| sql.contains(&edge.to_string()))),
        "neither {before} nor {after} bounds the window: {sql}"
    );
    assert!(
        !sql.contains("1757462400000"),
        "the upstream's newest row is 2025-09-10; its day must not bound the window: {sql}"
    );

    server.abort();
    Ok(())
}

#[tokio::test]
async fn a_run_that_wants_a_total_asks_for_the_window_without_a_bucket() -> R {
    let (address, server, seen) = recording_upstream().await;
    let harness = TestHarness::new(&address).await;

    let response = harness
        .ask(
            "opened",
            Some(clocked_metric()),
            Some(json!({"range": "P30D", "bucket": false})),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    let sql = only_read(&seen);
    assert!(!sql.contains("`bucket`"), "{sql}");
    assert!(sql.contains("'occurred_at'"), "{sql}");

    server.abort();
    Ok(())
}

#[tokio::test]
async fn a_window_wider_than_the_metric_allows_is_a_bad_request() -> R {
    let (address, server) = clocked_upstream().await;
    let harness = TestHarness::new(&address).await;
    let capped = json!({
        "dataset": "commits",
        "time": { "field": "occurred_at" },
        "max_range": "P7D",
        "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
    });

    let response = harness
        .ask("capped", Some(capped), Some(json!({"range": "P30D"})))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    server.abort();
    Ok(())
}
