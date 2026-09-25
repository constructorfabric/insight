use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine as _;
use serde_json::json;
use tower::ServiceExt as _;

use super::*;
use crate::api::AppState;
use crate::chat::ChatClient;
use crate::domain::definition::{DefinitionKind, Definitions};
use crate::domain::query::metric_query::MetricRunner;
use crate::store::definitions::memory::MemoryDefinitions;

type R = Result<(), Box<dyn std::error::Error>>;
type Seen = Arc<Mutex<Vec<String>>>;

struct TestHarness {
    router: Router,
    definitions: Arc<dyn Definitions>,
}

impl TestHarness {
    fn new(metrics_url: &str) -> Self {
        let openapi = toolkit::api::OpenApiRegistryImpl::new();
        let metrics_client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            metrics_url,
            "insight",
        ));
        let definitions: Arc<dyn Definitions> = Arc::new(MemoryDefinitions::new());
        let datasets = crate::api::Datasets::holding(
            metrics_url,
            &[(
                "commits",
                json!({
                    "title": "Commits",
                    "fields": [
                        { "name": "day", "path": "day", "type": "string" },
                        { "name": "lines", "path": "lines", "type": "int" },
                        { "name": "occurred_at", "path": "occurred_at", "type": "datetime" }
                    ]
                }),
            )],
        );
        let state = Arc::new(AppState::new(
            MetricRunner::new(
                metrics_client,
                crate::domain::query::metric_query::People::new("identity"),
            ),
            definitions.clone(),
            ChatClient::keyless(),
            crate::store::identity::IdentityClient::fixed(true),
            datasets,
            crate::store::catalog::Catalog::fixed(Vec::new()),
        ));
        let router = register_routes(Router::new(), &openapi, state);

        Self {
            router,
            definitions,
        }
    }

    async fn store(&self, name: &str, body: serde_json::Value) {
        let parsed = DefinitionName::parse(name)
            .unwrap_or_else(|error| panic!("test name must parse: {error}"));
        self.definitions
            .put(DefinitionKind::Metric, &parsed, &body)
            .await
            .unwrap_or_else(|error| panic!("the store must accept it: {error}"));
    }

    async fn page(&self, name: &str, asked: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let request = Request::builder()
            .method("POST")
            .uri(format!("/v1/metrics/{name}/drilldown"))
            .header("content-type", "application/json")
            .body(Body::from(asked.to_string()))
            .unwrap_or_else(|error| panic!("test request must be valid: {error}"));
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .unwrap_or_else(|error| panic!("router must respond: {error}"));
        let status = response.status();
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap_or_else(|error| panic!("response body must be readable: {error}"));
        let json = serde_json::from_slice(&body)
            .unwrap_or_else(|error| panic!("response body must be JSON: {error}"));

        (status, json)
    }
}

/// A warehouse that answers three rows to a first page and one to a resumed
/// one, and remembers what it was asked. The key's cells come back as the
/// wrapper projects them: the flag, the sorted cell, then every column.
async fn upstream() -> (String, Seen) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("the upstream must bind: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("the upstream must have an address: {error}"));
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = seen.clone();
    let router = Router::new().route(
        "/",
        post(move |sql: String| {
            let recorded = recorded.clone();
            async move {
                let resumed = sql.contains(" WHERE tuple(");
                if let Ok(mut recorded) = recorded.lock() {
                    recorded.push(sql);
                }
                let row = |day: &str, total: &str| {
                    json!({
                        "day": day, "total": total,
                        "__drilldown_flag": 0, "__drilldown_key": day,
                        "__drilldown_tie0": day, "__drilldown_tie1": total
                    })
                };
                let data = if resumed {
                    vec![row("2026-09-03", "1")]
                } else {
                    vec![
                        row("2026-09-01", "2"),
                        row("2026-09-02", "5"),
                        row("2026-09-03", "1"),
                    ]
                };
                Json(json!({ "meta": [], "data": data }))
            }
        }),
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    (format!("http://{address}"), seen)
}

fn lines_per_day() -> serde_json::Value {
    json!({
        "dataset": "commits",
        "fields": [
            { "field": "day", "type": "string", "as_name": "day" },
            { "agg": "count", "type": "int", "as_name": "total" }
        ],
        "group_by": ["day"]
    })
}

fn last_read(seen: &Seen) -> String {
    seen.lock()
        .unwrap_or_else(|error| panic!("the recording must be readable: {error}"))
        .last()
        .cloned()
        .unwrap_or_default()
}

fn field_of(body: &serde_json::Value) -> &str {
    body["context"]["field_violations"][0]["field"]
        .as_str()
        .unwrap_or_default()
}

#[tokio::test]
async fn a_first_page_is_ordered_by_the_warehouse_typed_and_cut_one_short_of_what_came_back() -> R {
    let (url, seen) = upstream().await;
    let harness = TestHarness::new(&url);
    harness.store("lines_per_day", lines_per_day()).await;

    let (status, body) = harness.page("lines_per_day", json!({ "limit": 2 })).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["columns"],
        json!([
            { "key": "day", "label": "day", "type": "string", "sortable": true, "percent": false },
            { "key": "total", "label": "total", "type": "number", "sortable": true, "percent": false }
        ])
    );
    assert_eq!(
        body["rows"],
        json!([
            { "values": { "day": "2026-09-01", "total": 2 } },
            { "values": { "day": "2026-09-02", "total": 5 } }
        ])
    );
    assert_eq!(
        body["selection"]["sort"],
        json!({ "key": "day", "direction": "asc" })
    );
    assert!(body["next_cursor"].is_string(), "{body}");

    let sql = last_read(&seen);
    assert!(sql.starts_with("SELECT __m.*, "), "{sql}");
    assert!(
        sql.contains(
            " GROUP BY `day`) AS __m ORDER BY `__drilldown_flag` ASC, `__drilldown_key` ASC, \
             `__drilldown_tie0` ASC, `__drilldown_tie1` ASC LIMIT 3"
        ),
        "{sql}"
    );

    Ok(())
}

#[tokio::test]
async fn the_next_page_resumes_past_the_cursor_and_ends_where_the_rows_do() -> R {
    let (url, seen) = upstream().await;
    let harness = TestHarness::new(&url);
    harness.store("lines_per_day", lines_per_day()).await;
    let (_, first) = harness.page("lines_per_day", json!({ "limit": 2 })).await;

    let (status, body) = harness
        .page(
            "lines_per_day",
            json!({ "limit": 2, "cursor": first["next_cursor"] }),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["rows"],
        json!([{ "values": { "day": "2026-09-03", "total": 1 } }])
    );
    assert!(body["next_cursor"].is_null(), "{body}");

    // The client writes the bound values into the statement it sends, so the
    // warehouse saw the cursor's key in tuple order: the flag, the sorted
    // cell, then every column as text.
    let sql = last_read(&seen);
    assert!(sql.contains(") AS __m WHERE tuple("), "{sql}");
    assert!(
        sql.contains(") > tuple(0, '2026-09-02', '2026-09-02', '5') ORDER BY"),
        "{sql}"
    );

    Ok(())
}

#[tokio::test]
async fn a_page_ordered_by_a_column_the_metric_does_not_produce_is_refused() -> R {
    let (url, _) = upstream().await;
    let harness = TestHarness::new(&url);
    harness.store("lines_per_day", lines_per_day()).await;

    let (status, body) = harness
        .page(
            "lines_per_day",
            json!({ "sort": { "key": "author", "direction": "asc" } }),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(field_of(&body), "sort.key");

    Ok(())
}

/// A cursor holds a position in one ordering over one metric. Replayed into
/// another it would serve a page out of a result that never existed.
#[tokio::test]
async fn a_cursor_issued_over_another_order_or_another_body_is_refused() -> R {
    let (url, _) = upstream().await;
    let harness = TestHarness::new(&url);
    harness.store("lines_per_day", lines_per_day()).await;
    let (_, first) = harness.page("lines_per_day", json!({ "limit": 2 })).await;
    let cursor = first["next_cursor"].clone();

    let (status, body) = harness
        .page(
            "lines_per_day",
            json!({ "limit": 2, "cursor": cursor, "sort": { "key": "total", "direction": "desc" } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(field_of(&body), "cursor");

    let mut edited = lines_per_day();
    edited["filters"] = json!([{ "field": "lines", "op": "gt", "value": 0, "type": "int" }]);
    harness.store("lines_per_day", edited).await;
    let (status, body) = harness
        .page("lines_per_day", json!({ "limit": 2, "cursor": cursor }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(field_of(&body), "cursor");

    Ok(())
}

/// The bytes are the caller's to edit. A key that no longer fits the order
/// is refused as the caller's mistake, not sent on to fail as ours.
#[tokio::test]
async fn a_cursor_whose_key_was_edited_is_refused_rather_than_run() -> R {
    let (url, _) = upstream().await;
    let harness = TestHarness::new(&url);
    harness.store("lines_per_day", lines_per_day()).await;
    let (_, first) = harness.page("lines_per_day", json!({ "limit": 2 })).await;

    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let bytes = engine.decode(first["next_cursor"].as_str().unwrap_or_default())?;
    let mut envelope: serde_json::Value = serde_json::from_slice(&bytes)?;
    envelope["key"]["ties"] = json!([]);
    let tampered = engine.encode(envelope.to_string());

    let (status, body) = harness
        .page("lines_per_day", json!({ "limit": 2, "cursor": tampered }))
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(field_of(&body), "cursor");

    Ok(())
}

#[tokio::test]
async fn what_cannot_be_asked_for_is_refused_by_name() -> R {
    let (url, _) = upstream().await;
    let harness = TestHarness::new(&url);
    harness.store("lines_per_day", lines_per_day()).await;
    let cases = [
        (
            "lines_per_day",
            json!({ "limit": 0 }),
            StatusCode::BAD_REQUEST,
            "limit",
        ),
        (
            "lines_per_day",
            json!({ "limit": 251 }),
            StatusCode::BAD_REQUEST,
            "limit",
        ),
        (
            "lines_per_day",
            json!({ "range": "P99Q" }),
            StatusCode::BAD_REQUEST,
            "range",
        ),
        ("nowhere", json!({}), StatusCode::NOT_FOUND, ""),
    ];

    for (metric, asked, expected, field) in cases {
        let (status, body) = harness.page(metric, asked.clone()).await;
        assert_eq!(status, expected, "{metric} {asked}: {body}");
        assert_eq!(field_of(&body), field, "{metric} {asked}: {body}");
    }

    Ok(())
}

/// `P7D` names a different seven days tomorrow. A walk is over one period,
/// so the second page reads the window the first one resolved.
#[tokio::test]
async fn a_continued_walk_reads_the_window_its_first_page_resolved() -> R {
    let (url, seen) = upstream().await;
    let harness = TestHarness::new(&url);
    let mut clocked = lines_per_day();
    clocked["time"] = json!({ "field": "occurred_at" });
    harness.store("lines_per_day", clocked).await;

    let (status, first) = harness
        .page(
            "lines_per_day",
            json!({ "limit": 2, "range": "P7D", "bucket": false }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first_read = last_read(&seen);

    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(first["next_cursor"].as_str().unwrap_or_default())?;
    let envelope: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert!(
        envelope["window"]["Requested"]["bounds"]["Finite"].is_object(),
        "{envelope}"
    );

    let (status, second) = harness
        .page(
            "lines_per_day",
            json!({ "limit": 2, "range": "P7D", "bucket": false, "cursor": first["next_cursor"] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");

    let window_of = |sql: &str| {
        let from = sql.find(" WHERE ").unwrap_or_default();
        let to = sql.find(" GROUP BY ").unwrap_or(sql.len());
        sql[from..to].to_owned()
    };
    assert_eq!(window_of(&first_read), window_of(&last_read(&seen)));

    Ok(())
}
