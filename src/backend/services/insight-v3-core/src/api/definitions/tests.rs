use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use clickhouse::test::Mock;
use serde_json::json;
use toolkit::api::OpenApiRegistryImpl;
use tower::ServiceExt as _;

use super::*;
use crate::api::AppState;
use crate::chat::ChatClient;
use crate::domain::definition::Definitions as _;
use crate::domain::folders::DefinitionStore;
use crate::domain::query::metric_query::MetricRunner;
use crate::store::definitions::memory::MemoryDefinitions;

/// The dataset every metric stored here reads.
fn commits() -> serde_json::Value {
    json!({
        "title": "Commits",
        "fields": [
            { "name": "day", "path": "day", "type": "string" },
            { "name": "committed_at", "path": "committed_at", "type": "datetime" },
            { "name": "lines_added", "path": "lines_added", "type": "int" }
        ]
    })
}

/// A dataset that marks a main date, so a metric over it is windowed without
/// naming a clock of its own.
fn deploys() -> serde_json::Value {
    json!({
        "title": "Deploys",
        "fields": [
            { "name": "service", "path": "service", "type": "string" },
            {
                "name": "deployed_at",
                "path": "deployed_at",
                "type": "datetime",
                "default_clock": true
            }
        ]
    })
}

struct Chain {
    metric: String,
    widget: String,
    dashboard: String,
}

struct TestHarness {
    /// Held, not read: the stores this harness does not exercise are built
    /// against its address, so it has to outlive them.
    _clickhouse: Mock,
    router: Router,
}

impl TestHarness {
    async fn new() -> Self {
        Self::with_caller(true).await
    }

    /// A harness whose definition store refuses every write.
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "the harness mirrors the async one"
    )]
    async fn with_a_store_that_is_down() -> Self {
        Self::build(
            true,
            &(Arc::new(MemoryDefinitions::refusing()) as Arc<dyn DefinitionStore>),
        )
    }

    /// A caller who does or does not hold the admin role.
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "the harness mirrors the async one"
    )]
    async fn with_caller(is_admin: bool) -> Self {
        Self::build(
            is_admin,
            &(Arc::new(MemoryDefinitions::new()) as Arc<dyn DefinitionStore>),
        )
    }

    fn build(is_admin: bool, definitions: &Arc<dyn DefinitionStore>) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let openapi = OpenApiRegistryImpl::new();
        let url = mock.url();
        let state = Arc::new(AppState::new(
            MetricRunner::new(
                insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "insight")),
                crate::domain::query::metric_query::People::new("identity"),
            ),
            definitions.clone(),
            ChatClient::keyless(),
            crate::store::identity::IdentityClient::fixed(is_admin),
            crate::api::Datasets::holding(url, &[("commits", commits()), ("deploys", deploys())]),
            crate::store::catalog::Catalog::fixed(Vec::new()),
        ));
        let router = register_routes(Router::new(), &openapi, &state);

        Self {
            _clickhouse: mock,
            router,
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

    async fn post_json(&self, path: &str, body: serde_json::Value) -> TestResponse {
        let request = Request::builder()
            .method("POST")
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

    /// Creates a definition and answers the id it was given.
    async fn created(&self, segment: &str, name: &str, body: serde_json::Value) -> String {
        let made = self
            .post_json(
                &format!("/v1/{segment}"),
                json!({ "name": name, "body": body }),
            )
            .await;
        assert_eq!(made.status(), StatusCode::CREATED, "{segment} {name}");
        let answered = made.json().await;

        answered["id"]
            .as_str()
            .unwrap_or_else(|| panic!("a create answers its id: {answered}"))
            .to_owned()
    }

    async fn replaced(
        &self,
        segment: &str,
        id: &str,
        name: &str,
        body: serde_json::Value,
    ) -> TestResponse {
        self.put_json(
            &format!("/v1/{segment}/{id}"),
            json!({ "name": name, "body": body }),
        )
        .await
    }

    /// A metric, a widget drawing it, and a dashboard holding that widget,
    /// by the ids they were given.
    async fn seed_chain(&self) -> Chain {
        let metric = self
            .created(
                "metrics",
                "Commits per day",
                json!({
                    "dataset": "commits",
                    "fields": [{ "field": "day", "type": "string", "as_name": "day" }]
                }),
            )
            .await;
        let widget = self
            .created(
                "widgets",
                "Commits table",
                json!({ "type": "table", "metric": metric, "columns": ["day"] }),
            )
            .await;
        let dashboard = self
            .created(
                "dashboards",
                "Engineering",
                json!({ "title": "Engineering", "widgets": [widget] }),
            )
            .await;

        Chain {
            metric,
            widget,
            dashboard,
        }
    }

    async fn get_json(&self, path: &str) -> TestResponse {
        let request = Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
            .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .unwrap_or_else(|error| panic!("router must respond: {error}"));

        TestResponse::from_response(response).await
    }

    async fn delete_json(&self, path: &str) -> TestResponse {
        let request = Request::builder()
            .method("DELETE")
            .uri(path)
            .body(Body::empty())
            .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .unwrap_or_else(|error| panic!("router must respond: {error}"));

        TestResponse::from_response(response).await
    }

    async fn list_json(&self, path: &str) -> TestResponse {
        let request = Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
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

/// The smallest body that reads back as a metric, for tests about storage
/// rather than about what a metric says.
fn metric_body() -> serde_json::Value {
    json!({
        "dataset": "commits",
        "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
    })
}

fn day_metric() -> serde_json::Value {
    json!({
        "dataset": "commits",
        "fields": [{ "field": "day", "type": "string", "as_name": "day" }]
    })
}

/// The items a listing answers, in its order.
fn items(listed: &serde_json::Value) -> &[serde_json::Value] {
    listed["items"]
        .as_array()
        .unwrap_or_else(|| panic!("a listing answers items: {listed}"))
}

/// The names a listing answers, in its order.
fn names(listed: &serde_json::Value) -> Vec<String> {
    items(listed)
        .iter()
        .map(|item| item["name"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// A stamp written as UTC to the millisecond: `YYYY-MM-DDTHH:mm:ss.sssZ`.
fn assert_millisecond_utc(stamp: &serde_json::Value, case: &str) {
    let written = stamp.as_str().unwrap_or_default();
    let shape = chrono::NaiveDateTime::parse_from_str(written, "%Y-%m-%dT%H:%M:%S%.3fZ");

    assert!(
        shape.is_ok() && written.len() == "2026-01-01T00:00:00.000Z".len(),
        "{case}: `{written}` should read YYYY-MM-DDTHH:mm:ss.sssZ"
    );
}

/// A reader is shown what a removal would break before they ask for one.
#[tokio::test]
async fn a_definition_names_what_holds_it() {
    let harness = TestHarness::new().await;
    let metric = harness
        .created("metrics", "Commits per day", metric_body())
        .await;
    let widget = harness
        .created(
            "widgets",
            "Commits table",
            json!({ "type": "table", "metric": metric, "columns": ["total"] }),
        )
        .await;

    let held = harness
        .get_json(&format!("/v1/metrics/{metric}/dependents"))
        .await;

    assert_eq!(held.status(), StatusCode::OK);
    assert_eq!(
        held.json().await["holders"],
        json!([{ "kind": "widgets", "id": widget, "name": "Commits table" }])
    );
}

#[tokio::test]
async fn a_definition_nothing_holds_names_nothing() {
    let harness = TestHarness::new().await;
    let metric = harness
        .created("metrics", "Commits per day", metric_body())
        .await;

    let held = harness
        .get_json(&format!("/v1/metrics/{metric}/dependents"))
        .await;

    assert_eq!(held.json().await["holders"], json!([]));
}

#[tokio::test]
async fn the_dependents_of_a_definition_that_is_not_there_are_not_found() {
    let harness = TestHarness::new().await;

    let held = harness.get_json("/v1/metrics/nobody/dependents").await;

    assert_eq!(held.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_create_answers_an_id_that_reads_back_with_its_name_and_body() {
    let harness = TestHarness::new().await;

    let id = harness
        .created("metrics", "Commits per day", metric_body())
        .await;

    let got = harness.get_json(&format!("/v1/metrics/{id}")).await;
    assert_eq!(got.status(), StatusCode::OK);
    let read = got.json().await;
    assert_eq!(read["id"], json!(id));
    assert_eq!(read["name"], "Commits per day");
    assert_eq!(read["body"], metric_body());
}

#[tokio::test]
async fn two_definitions_may_share_a_name_and_each_keeps_its_own_id() {
    let harness = TestHarness::new().await;

    let first = harness.created("metrics", "Lines", metric_body()).await;
    let second = harness.created("metrics", "Lines", metric_body()).await;

    assert_ne!(first, second);
    let listed = harness.list_json("/v1/metrics").await.json().await;
    assert_eq!(listed["total"], json!(2));
    assert_eq!(names(&listed), ["Lines", "Lines"]);
}

#[tokio::test]
async fn a_replace_changes_the_name_and_the_body_under_the_same_id() {
    let harness = TestHarness::new().await;
    let id = harness.created("metrics", "Before", metric_body()).await;

    let replaced = harness
        .replaced("metrics", &id, "After", day_metric())
        .await;

    assert_eq!(replaced.status(), StatusCode::NO_CONTENT);
    let read = harness
        .get_json(&format!("/v1/metrics/{id}"))
        .await
        .json()
        .await;
    assert_eq!(
        (&read["name"], &read["body"]),
        (&json!("After"), &day_metric())
    );
}

#[tokio::test]
async fn a_replace_of_an_id_nobody_holds_is_not_found_and_stores_nothing() {
    let harness = TestHarness::new().await;

    let refused = harness
        .replaced("metrics", "nowhere", "Lines", metric_body())
        .await;

    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    let listed = harness.list_json("/v1/metrics").await.json().await;
    assert_eq!(listed["total"], json!(0));
}

#[tokio::test]
async fn a_name_that_is_blank_or_too_long_is_refused() {
    let harness = TestHarness::new().await;
    let too_long = "n".repeat(129);

    for name in ["", "   ", too_long.as_str()] {
        let refused = harness
            .post_json(
                "/v1/metrics",
                json!({ "name": name, "body": metric_body() }),
            )
            .await;

        assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "for {name:?}");
    }
}

/// An id that cannot be one names nothing, so every route answers it as it
/// answers an id nobody holds.
#[tokio::test]
async fn a_malformed_id_in_a_path_is_not_found() {
    let harness = TestHarness::new().await;
    let malformed = "/v1/metrics/drop%20table";

    let read = harness.get_json(malformed).await;
    let replaced = harness
        .put_json(malformed, json!({ "name": "Lines", "body": metric_body() }))
        .await;
    let removed = harness.delete_json(malformed).await;
    let held = harness
        .get_json("/v1/metrics/drop%20table/dependents")
        .await;

    for (what, answered) in [
        ("read", read),
        ("replace", replaced),
        ("delete", removed),
        ("dependents", held),
    ] {
        assert_eq!(answered.status(), StatusCode::NOT_FOUND, "{what}");
    }
}

/// A card is windowed or not by what the read says, never by whether the
/// stored body happens to name a clock.
#[tokio::test]
async fn a_metric_inheriting_its_dataset_s_clock_reads_back_saying_so() {
    let harness = TestHarness::new().await;
    let id = harness
        .created(
            "metrics",
            "Deploys per day",
            json!({
                "dataset": "deploys",
                "fields": [{"agg": "count", "type": "int", "as_name": "total"}]
            }),
        )
        .await;

    let got = harness.get_json(&format!("/v1/metrics/{id}")).await;

    assert_eq!(
        got.json().await["clock"],
        json!({ "field": "deployed_at", "from": "dataset" })
    );
}

#[tokio::test]
async fn a_metric_naming_its_own_clock_reads_back_saying_it_named_it() {
    let harness = TestHarness::new().await;
    let id = harness
        .created(
            "metrics",
            "Commits per day",
            json!({
                "dataset": "commits",
                "time": {"field": "committed_at", "type": "datetime"},
                "fields": [{"agg": "count", "type": "int", "as_name": "total"}]
            }),
        )
        .await;

    let got = harness.get_json(&format!("/v1/metrics/{id}")).await;

    assert_eq!(
        got.json().await["clock"],
        json!({ "field": "committed_at", "from": "metric" })
    );
}

/// A dataset marking no main date leaves a metric that names none unwindowed,
/// and the read says nothing rather than inventing a field.
#[tokio::test]
async fn a_metric_with_no_clock_anywhere_reads_back_without_one() {
    let harness = TestHarness::new().await;
    let id = harness
        .created("metrics", "All commits", metric_body())
        .await;

    let got = harness.get_json(&format!("/v1/metrics/{id}")).await;

    assert_eq!(got.json().await["clock"], json!(null));
}

#[tokio::test]
async fn get_missing_definition_is_not_found() {
    let harness = TestHarness::new().await;

    let got = harness.get_json("/v1/metrics/nope").await;
    assert_eq!(got.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_answers_each_definition_by_id_and_name_in_name_order() {
    let harness = TestHarness::new().await;

    // Stored out of order, listed in it.
    let lines = harness
        .created("metrics", "Lines per day", metric_body())
        .await;
    let commits = harness
        .created("metrics", "Commits per day", metric_body())
        .await;

    let got = harness.list_json("/v1/metrics").await;

    assert_eq!(got.status(), StatusCode::OK);
    let listed = got.json().await;
    let ids: Vec<&str> = items(&listed)
        .iter()
        .map(|item| item["id"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(ids, [commits.as_str(), lines.as_str()], "{listed}");
    assert_eq!(names(&listed), ["Commits per day", "Lines per day"]);
    assert_eq!(
        (&listed["total"], &listed["limit"], &listed["offset"]),
        (&json!(2), &json!(50), &json!(0))
    );
}

#[tokio::test]
async fn a_catalogue_item_carries_its_body_and_when_it_last_changed() {
    let harness = TestHarness::new().await;
    let chain = harness.seed_chain().await;
    let cases = [
        (
            "metrics",
            chain.metric.as_str(),
            json!({
                "dataset": "commits",
                "fields": [{ "field": "day", "type": "string", "as_name": "day" }]
            }),
        ),
        (
            "widgets",
            chain.widget.as_str(),
            json!({ "type": "table", "metric": chain.metric, "columns": ["day"] }),
        ),
    ];

    for (segment, id, body) in cases {
        let listed = harness
            .list_json(&format!("/v1/{segment}"))
            .await
            .json()
            .await;
        let [item] = items(&listed) else {
            panic!("{segment} should list one item: {listed}");
        };

        assert_eq!(item["id"], json!(id), "{segment}: {item}");
        assert_eq!(item["body"], body, "{segment}: {item}");
        assert_millisecond_utc(&item["updated_at"], segment);
    }
}

#[tokio::test]
async fn a_widget_item_names_the_metric_it_draws_and_null_for_one_that_is_gone() {
    let definitions = Arc::new(MemoryDefinitions::new());
    let store = Arc::clone(&definitions) as Arc<dyn DefinitionStore>;
    let harness = TestHarness::build(true, &store);
    let chain = harness.seed_chain().await;
    let orphan = DefinitionId::parse("orphan_widget")
        .unwrap_or_else(|error| panic!("the fixture id parses: {error}"));
    let label = DefinitionLabel::parse("Orphaned table")
        .unwrap_or_else(|error| panic!("the fixture name parses: {error}"));
    definitions
        .create(
            DefinitionKind::Widget,
            &orphan,
            &label,
            &json!({ "type": "table", "metric": "removed_metric", "columns": ["day"] }),
        )
        .await
        .unwrap_or_else(|error| panic!("the orphan widget is stored: {error}"));

    let listed = harness.list_json("/v1/widgets").await.json().await;

    let drawn: Vec<(&str, &serde_json::Value)> = items(&listed)
        .iter()
        .map(|item| {
            (
                item["id"].as_str().unwrap_or_default(),
                &item["metric_name"],
            )
        })
        .collect();
    assert_eq!(
        drawn,
        [
            (chain.widget.as_str(), &json!("Commits per day")),
            ("orphan_widget", &serde_json::Value::Null),
        ],
        "{listed}"
    );
}

#[tokio::test]
async fn a_metric_item_carries_no_metric_name() {
    let harness = TestHarness::new().await;
    harness.seed_chain().await;

    let listed = harness.list_json("/v1/metrics").await.json().await;

    let [item] = items(&listed) else {
        panic!("one metric should be listed: {listed}");
    };
    assert!(item.get("metric_name").is_none(), "{item}");
}

#[tokio::test]
async fn a_widget_definition_round_trips() {
    let harness = TestHarness::new().await;

    // The widget draws this metric's columns, so it has to be there first.
    let metric = harness
        .created("metrics", "Commits per day", day_metric())
        .await;
    let drawn = json!({ "type": "table", "metric": metric, "columns": ["day"] });
    let widget = harness
        .created("widgets", "Commits table", drawn.clone())
        .await;

    let got = harness.get_json(&format!("/v1/widgets/{widget}")).await;
    assert_eq!(got.status(), StatusCode::OK);
    assert_eq!(got.json().await["body"], drawn);
}

#[tokio::test]
async fn a_caller_without_the_admin_role_reaches_nothing() {
    let harness = TestHarness::with_caller(false).await;
    let write = json!({ "name": "Chart", "body": { "type": "table", "metric": "m" } });

    let refused = [
        ("GET /v1/metrics", harness.list_json("/v1/metrics").await),
        (
            "GET /v1/metrics/commits_per_day",
            harness.get_json("/v1/metrics/commits_per_day").await,
        ),
        (
            "POST /v1/widgets",
            harness.post_json("/v1/widgets", write.clone()).await,
        ),
        (
            "PUT /v1/widgets/commits_table",
            harness.put_json("/v1/widgets/commits_table", write).await,
        ),
        (
            "DELETE /v1/metrics/anything",
            harness.delete_json("/v1/metrics/anything").await,
        ),
    ];

    for (asked, response) in refused {
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{asked} must refuse a caller without the role"
        );
    }
}

#[tokio::test]
async fn a_definition_nothing_uses_is_removed() {
    let harness = TestHarness::new().await;
    let id = harness.created("metrics", "Spare", day_metric()).await;

    let removed = harness.delete_json(&format!("/v1/metrics/{id}")).await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    let listed = harness.list_json("/v1/metrics").await;
    assert_eq!(
        listed.json().await,
        json!({ "items": [], "total": 0, "limit": 50, "offset": 0 })
    );
}

#[tokio::test]
async fn removing_what_is_not_there_says_so() {
    let harness = TestHarness::new().await;

    let removed = harness.delete_json("/v1/dashboards/never_existed").await;

    assert_eq!(removed.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_metric_a_widget_draws_is_kept_and_the_widget_named() {
    let harness = TestHarness::new().await;
    let metric = harness
        .created("metrics", "Lines per day", day_metric())
        .await;
    harness
        .created(
            "widgets",
            "Lines table",
            json!({ "type": "table", "metric": metric, "columns": ["day"] }),
        )
        .await;

    let refused = harness.delete_json(&format!("/v1/metrics/{metric}")).await;

    // A widget whose metric is gone renders an error where a chart should be.
    // Something still holding it is a conflict, the same answer a dataset
    // something still reads gives, and the reply names the widget.
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    let body = refused.json().await;
    assert!(
        body.to_string().contains("Lines table"),
        "the refusal must name what still uses it: {body}"
    );
    // And it really is still there.
    assert_eq!(
        harness
            .get_json(&format!("/v1/metrics/{metric}"))
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_widget_a_dashboard_holds_is_kept_and_the_dashboard_named() {
    let harness = TestHarness::new().await;
    let ids = harness.seed_chain().await;

    let refused = harness
        .delete_json(&format!("/v1/widgets/{}", ids.widget))
        .await;

    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert!(refused.json().await.to_string().contains("Engineering"));
}

#[tokio::test]
async fn a_dashboard_is_removed_even_though_widgets_point_into_it() {
    // Nothing holds a dashboard, so it is always free to remove - and its
    // widgets and metrics stay for the next one.
    let harness = TestHarness::new().await;
    let ids = harness.seed_chain().await;

    let removed = harness
        .delete_json(&format!("/v1/dashboards/{}", ids.dashboard))
        .await;

    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        harness
            .get_json(&format!("/v1/metrics/{}", ids.metric))
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_search_matches_a_name() {
    let harness = TestHarness::new().await;
    harness.seed_chain().await;

    let found = harness.get_json("/v1/metrics?q=per%20day").await;

    assert_eq!(found.status(), StatusCode::OK);
    assert_eq!(names(&found.json().await), ["Commits per day"]);
}

#[tokio::test]
async fn a_search_matches_what_the_body_says() {
    // "Which metrics read this field" is the question a catalogue is asked,
    // and a name cannot answer it.
    let harness = TestHarness::new().await;
    harness.seed_chain().await;
    harness
        .created(
            "metrics",
            "Lines per day",
            json!({
                "dataset": "commits",
                "fields": [{ "field": "lines_added", "type": "int", "as_name": "lines" }]
            }),
        )
        .await;

    let found = harness.get_json("/v1/metrics?q=lines_added").await;

    assert_eq!(names(&found.json().await), ["Lines per day"]);
}

#[tokio::test]
async fn an_empty_search_is_not_a_search_for_nothing() {
    let harness = TestHarness::new().await;
    harness.seed_chain().await;

    let all = harness.get_json("/v1/widgets?q=").await;

    assert_eq!(names(&all.json().await), ["Commits table"]);
}

#[tokio::test]
async fn a_search_that_matches_nothing_is_an_empty_list() {
    let harness = TestHarness::new().await;
    harness.seed_chain().await;

    let none = harness.get_json("/v1/dashboards?q=nothing_like_this").await;

    assert_eq!(none.status(), StatusCode::OK);
    assert_eq!(none.json().await["items"], json!([]));
}

#[tokio::test]
async fn a_list_answers_one_page_and_how_many_there_are() {
    let harness = TestHarness::new().await;
    for name in ["a one", "b two", "c three"] {
        harness.created("metrics", name, metric_body()).await;
    }

    let page = harness.list_json("/v1/metrics?limit=2&offset=1").await;

    assert_eq!(page.status(), StatusCode::OK);
    let listed = page.json().await;
    assert_eq!(names(&listed), ["b two", "c three"]);
    assert_eq!(
        (&listed["total"], &listed["limit"], &listed["offset"]),
        (&json!(3), &json!(2), &json!(1))
    );
}

#[tokio::test]
async fn a_page_bigger_than_the_cap_is_refused() {
    let harness = TestHarness::new().await;

    let refused = harness.list_json("/v1/metrics?limit=5000").await;

    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_metric_body_that_cannot_be_read_as_a_query_is_refused_on_write() {
    let harness = TestHarness::new().await;

    let refusal = harness
        .post_json(
            "/v1/metrics",
            json!({ "name": "Commits", "body": { "dataset": "commits" } }),
        )
        .await;

    assert_eq!(refusal.status, StatusCode::BAD_REQUEST);
    let listed = harness.list_json("/v1/metrics").await.json().await;
    assert_eq!(
        listed["total"],
        json!(0),
        "a refused body must not be stored"
    );
}

#[tokio::test]
async fn a_metric_offering_a_window_the_server_cannot_resolve_is_refused_on_write() {
    let harness = TestHarness::new().await;
    let windowed = |max_range: &str| {
        json!({
            "name": "Commits",
            "body": {
                "dataset": "commits",
                "fields": [{ "field": "day", "type": "string", "as_name": "day" }],
                "time": { "field": "committed_at" },
                "max_range": max_range,
            }
        })
    };

    let refusal = harness.post_json("/v1/metrics", windowed("for ever")).await;
    let sound = harness.post_json("/v1/metrics", windowed("P6M")).await;

    assert_eq!(refusal.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        sound.status,
        StatusCode::CREATED,
        "only the window the server cannot resolve is the caller's mistake"
    );
    let listed = harness.list_json("/v1/metrics").await.json().await;
    assert_eq!(listed["total"], json!(1));
}

#[tokio::test]
async fn a_store_that_is_down_is_ours_to_fix_and_says_nothing_about_itself() {
    let harness = TestHarness::with_a_store_that_is_down().await;

    let failed = harness
        .post_json(
            "/v1/metrics",
            json!({ "name": "Commits", "body": day_metric() }),
        )
        .await;

    assert_eq!(failed.status, StatusCode::INTERNAL_SERVER_ERROR);
    let said = String::from_utf8_lossy(&failed.body);
    assert!(!said.contains("store is down"), "{said}");
}

async fn last_written(harness: &TestHarness, path: &str) -> chrono::DateTime<chrono::Utc> {
    let read = harness.get_json(path).await;
    assert_eq!(read.status(), StatusCode::OK);
    let body = read.json().await;
    let stamp = body["updated_at"].as_str().unwrap_or_default().to_owned();

    assert!(stamp.ends_with('Z'), "a UTC stamp: {body}");
    chrono::DateTime::parse_from_rfc3339(&stamp)
        .unwrap_or_else(|error| panic!("`{stamp}` must be RFC 3339: {error}"))
        .with_timezone(&chrono::Utc)
}

#[tokio::test]
async fn a_dashboard_read_says_when_it_was_last_written_and_a_write_moves_it() {
    let harness = TestHarness::new().await;
    let before = chrono::SubsecRound::trunc_subsecs(chrono::Utc::now(), 3);
    let id = harness
        .created(
            "dashboards",
            "Board",
            json!({ "title": "Board", "widgets": [] }),
        )
        .await;
    let path = format!("/v1/dashboards/{id}");

    let first = last_written(&harness, &path).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    harness
        .replaced(
            "dashboards",
            &id,
            "Board",
            json!({ "title": "Board, again", "widgets": [] }),
        )
        .await;
    let second = last_written(&harness, &path).await;

    assert!(first >= before, "{first} should not precede {before}");
    assert!(second > first, "{second} should follow {first}");
}

#[tokio::test]
async fn a_dashboard_stamp_is_utc_to_the_millisecond() {
    let harness = TestHarness::new().await;
    let id = harness
        .created(
            "dashboards",
            "Board",
            json!({ "title": "Board", "widgets": [] }),
        )
        .await;

    let read = harness
        .get_json(&format!("/v1/dashboards/{id}"))
        .await
        .json()
        .await;
    let stamp = read["updated_at"].as_str().unwrap_or_default();

    let shape = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%dT%H:%M:%S%.3fZ");
    assert!(
        shape.is_ok() && stamp.len() == "2026-01-01T00:00:00.000Z".len(),
        "`{stamp}` should read YYYY-MM-DDTHH:mm:ss.sssZ"
    );
}
