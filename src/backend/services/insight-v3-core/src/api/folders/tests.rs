use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use clickhouse::test::Mock;
use serde_json::{Value, json};
use toolkit::api::OpenApiRegistryImpl;
use tower::ServiceExt as _;

use crate::api::AppState;
use crate::chat::ChatClient;
use crate::domain::folders::DefinitionStore;
use crate::domain::query::metric_query::{MetricRunner, People};
use crate::store::definitions::memory::MemoryDefinitions;

fn commits() -> Value {
    json!({
        "title": "Commits",
        "fields": [{ "name": "day", "path": "day", "type": "string" }]
    })
}

struct Harness {
    _clickhouse: Mock,
    router: Router,
}

struct Answer {
    status: StatusCode,
    body: Value,
}

impl Harness {
    fn new() -> Self {
        Self::build(true, Arc::new(MemoryDefinitions::new()))
    }

    fn build(is_admin: bool, store: Arc<dyn DefinitionStore>) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let url = mock.url();
        let openapi = OpenApiRegistryImpl::new();
        let state = Arc::new(AppState::new(
            MetricRunner::new(
                insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "insight")),
                People::new("identity"),
            ),
            store,
            ChatClient::keyless(),
            crate::store::identity::IdentityClient::fixed(is_admin),
            crate::api::Datasets::holding(url, &[("commits", commits())]),
            crate::store::catalog::Catalog::fixed(Vec::new()),
        ));

        let router = crate::api::definitions::register_routes(Router::new(), &openapi, &state);
        let router = super::register_routes(router, &openapi, &state);

        Self {
            _clickhouse: mock,
            router,
        }
    }

    async fn send(&self, method: &str, path: &str, body: Option<Value>) -> Answer {
        let request = Request::builder().method(method).uri(path);
        let request = match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .unwrap_or_else(|error| panic!("router must respond: {error}"));
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap_or_else(|error| panic!("response body must be readable: {error}"));
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|error| panic!("response body must be JSON: {error}"))
        };

        Answer { status, body }
    }

    async fn create(&self, name: &str) -> Answer {
        self.send("POST", "/v1/folders", Some(json!({ "name": name })))
            .await
    }

    async fn created_id(&self, name: &str) -> String {
        let answer = self.create(name).await;
        assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.body);

        answer.body["id"]
            .as_str()
            .unwrap_or_else(|| panic!("a created folder names its id: {}", answer.body))
            .to_owned()
    }

    async fn dashboard(&self, name: &str) {
        let answer = self
            .send(
                "PUT",
                &format!("/v1/dashboards/{name}"),
                Some(json!({ "title": name, "widgets": [] })),
            )
            .await;
        assert_eq!(answer.status, StatusCode::NO_CONTENT, "{}", answer.body);
    }

    async fn file(&self, dashboard: &str, folder: Value) -> Answer {
        self.send(
            "PUT",
            &format!("/v1/dashboards/{dashboard}/folder"),
            Some(json!({ "folder": folder })),
        )
        .await
    }

    async fn folder_of(&self, dashboard: &str) -> Value {
        let answer = self
            .send("GET", &format!("/v1/dashboards/{dashboard}"), None)
            .await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);

        answer.body["folder"].clone()
    }
}

#[tokio::test]
async fn a_created_folder_is_listed_holding_nothing() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;

    let created = harness.create("Platform").await;
    let listed = harness.send("GET", "/v1/folders", None).await;

    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.body["name"], "Platform");
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(
        listed.body,
        json!({
            "folders": [{ "id": created.body["id"], "name": "Platform", "dashboards": 0 }],
            "unfiled": 1
        })
    );
}

#[tokio::test]
async fn a_name_is_kept_without_the_spaces_around_it() {
    let harness = Harness::new();

    let created = harness.create("  Platform ").await;

    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.body["name"], "Platform");
}

#[tokio::test]
async fn a_name_differing_only_in_case_is_taken() {
    let harness = Harness::new();
    harness.created_id("Platform").await;

    let clash = harness.create("platform").await;

    assert_eq!(clash.status, StatusCode::CONFLICT, "{}", clash.body);
}

#[tokio::test]
async fn an_empty_blank_or_overlong_name_is_refused() {
    let harness = Harness::new();

    for name in [String::new(), "   ".to_owned(), "x".repeat(65)] {
        let refused = harness.create(&name).await;

        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{name:?}");
    }
    assert_eq!(
        harness.create(&"x".repeat(64)).await.status,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn a_body_that_is_not_a_name_is_refused() {
    let harness = Harness::new();

    let refused = harness
        .send("POST", "/v1/folders", Some(json!({ "title": "Platform" })))
        .await;

    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_renamed_folder_keeps_its_id_and_its_dashboards() {
    let harness = Harness::new();
    let id = harness.created_id("Platform").await;
    harness.dashboard("delivery").await;
    harness.file("delivery", json!(id)).await;

    let renamed = harness
        .send(
            "PATCH",
            &format!("/v1/folders/{id}"),
            Some(json!({ "name": "Platform team" })),
        )
        .await;

    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    assert_eq!(renamed.body, json!({ "id": id, "name": "Platform team" }));
    assert_eq!(
        harness.folder_of("delivery").await,
        json!({ "id": id, "name": "Platform team" })
    );
}

#[tokio::test]
async fn renaming_onto_another_folder_s_name_is_taken() {
    let harness = Harness::new();
    harness.created_id("Platform").await;
    let product = harness.created_id("Product").await;

    let clash = harness
        .send(
            "PATCH",
            &format!("/v1/folders/{product}"),
            Some(json!({ "name": "PLATFORM" })),
        )
        .await;

    assert_eq!(clash.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_folder_id_that_names_nothing_is_not_found() {
    let harness = Harness::new();
    let gone = harness.created_id("Gone").await;
    harness
        .send("DELETE", &format!("/v1/folders/{gone}"), None)
        .await;

    let renamed = harness
        .send(
            "PATCH",
            &format!("/v1/folders/{gone}"),
            Some(json!({ "name": "Back" })),
        )
        .await;
    let removed = harness
        .send("DELETE", &format!("/v1/folders/{gone}"), None)
        .await;

    assert_eq!(renamed.status, StatusCode::NOT_FOUND);
    assert_eq!(removed.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_folder_id_that_is_not_a_uuid_is_refused() {
    let harness = Harness::new();

    let renamed = harness
        .send(
            "PATCH",
            "/v1/folders/platform",
            Some(json!({ "name": "Platform" })),
        )
        .await;
    let removed = harness.send("DELETE", "/v1/folders/platform", None).await;

    assert_eq!(renamed.status, StatusCode::BAD_REQUEST);
    assert_eq!(removed.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn deleting_a_folder_leaves_its_dashboards_unfiled() {
    let harness = Harness::new();
    let id = harness.created_id("Platform").await;
    harness.dashboard("delivery").await;
    harness.file("delivery", json!(id)).await;

    let removed = harness
        .send("DELETE", &format!("/v1/folders/{id}"), None)
        .await;

    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(harness.folder_of("delivery").await, Value::Null);
    assert_eq!(
        harness.send("GET", "/v1/folders", None).await.body,
        json!({ "folders": [], "unfiled": 1 })
    );
}

#[tokio::test]
async fn a_dashboard_moved_into_a_folder_reads_back_in_it_with_its_body_intact() {
    let harness = Harness::new();
    let id = harness.created_id("Platform").await;
    harness.dashboard("delivery").await;

    let moved = harness.file("delivery", json!(id)).await;
    let read = harness.send("GET", "/v1/dashboards/delivery", None).await;

    assert_eq!(moved.status, StatusCode::NO_CONTENT, "{}", moved.body);
    assert_eq!(read.body["folder"], json!({ "id": id, "name": "Platform" }));
    assert_eq!(
        read.body["body"],
        json!({ "title": "delivery", "widgets": [] })
    );
}

#[tokio::test]
async fn a_rewritten_dashboard_stays_in_its_folder() {
    let harness = Harness::new();
    let id = harness.created_id("Platform").await;
    harness.dashboard("delivery").await;
    harness.file("delivery", json!(id)).await;

    harness.dashboard("delivery").await;

    assert_eq!(harness.folder_of("delivery").await["id"], json!(id));
}

#[tokio::test]
async fn a_dashboard_moved_to_no_folder_is_unfiled() {
    let harness = Harness::new();
    let id = harness.created_id("Platform").await;
    harness.dashboard("delivery").await;
    harness.file("delivery", json!(id)).await;

    let moved = harness.file("delivery", Value::Null).await;

    assert_eq!(moved.status, StatusCode::NO_CONTENT);
    assert_eq!(harness.folder_of("delivery").await, Value::Null);
}

#[tokio::test]
async fn a_move_that_names_no_folder_at_all_is_refused() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;

    let refused = harness
        .send(
            "PUT",
            "/v1/dashboards/delivery/folder",
            Some(json!({ "folderId": null })),
        )
        .await;

    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn moving_into_a_folder_that_went_away_is_not_found_and_moves_nothing() {
    let harness = Harness::new();
    let platform = harness.created_id("Platform").await;
    let gone = harness.created_id("Gone").await;
    harness.dashboard("delivery").await;
    harness.file("delivery", json!(platform)).await;
    harness
        .send("DELETE", &format!("/v1/folders/{gone}"), None)
        .await;

    let moved = harness.file("delivery", json!(gone)).await;

    assert_eq!(moved.status, StatusCode::NOT_FOUND, "{}", moved.body);
    assert_eq!(harness.folder_of("delivery").await["id"], json!(platform));
}

#[tokio::test]
async fn moving_a_dashboard_that_is_not_there_is_not_found() {
    let harness = Harness::new();
    let id = harness.created_id("Platform").await;

    let moved = harness.file("nowhere", json!(id)).await;

    assert_eq!(moved.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn moving_into_a_folder_id_that_is_not_a_uuid_is_refused() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;

    let moved = harness.file("delivery", json!("platform")).await;

    assert_eq!(moved.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_dashboard_list_narrows_to_one_folder_or_to_the_unfiled() {
    let harness = Harness::new();
    let id = harness.created_id("Platform").await;
    for name in ["delivery", "delivery_costs", "hiring"] {
        harness.dashboard(name).await;
    }
    harness.file("delivery", json!(id)).await;

    let filed = harness
        .send("GET", &format!("/v1/dashboards?folder={id}"), None)
        .await;
    let unfiled = harness
        .send("GET", "/v1/dashboards?folder=unfiled&q=%20deliv%20", None)
        .await;

    assert_eq!(filed.status, StatusCode::OK);
    assert_eq!(filed.body["names"], json!(["delivery"]));
    assert_eq!(filed.body["total"], 1);
    assert_eq!(unfiled.body["names"], json!(["delivery_costs"]));
    assert_eq!(unfiled.body["total"], 1);
}

#[tokio::test]
async fn a_dashboard_list_in_a_folder_that_is_not_a_uuid_is_refused() {
    let harness = Harness::new();

    let refused = harness
        .send("GET", "/v1/dashboards?folder=platform", None)
        .await;

    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn only_dashboards_are_listed_by_folder() {
    let harness = Harness::new();

    let refused = harness
        .send("GET", "/v1/metrics?folder=unfiled", None)
        .await;

    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_read_of_another_kind_names_no_folder() {
    let harness = Harness::new();
    harness
        .send(
            "PUT",
            "/v1/metrics/commit_count",
            Some(json!({
                "dataset": "commits",
                "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
            })),
        )
        .await;

    let read = harness.send("GET", "/v1/metrics/commit_count", None).await;

    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert!(read.body.get("folder").is_none(), "{}", read.body);
}

#[tokio::test]
async fn a_caller_without_the_admin_role_is_refused_on_every_folder_route() {
    let harness = Harness::build(false, Arc::new(MemoryDefinitions::new()));
    let id = "0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
    let routes = [
        ("GET", "/v1/folders".to_owned(), None),
        (
            "POST",
            "/v1/folders".to_owned(),
            Some(json!({ "name": "Platform" })),
        ),
        (
            "PATCH",
            format!("/v1/folders/{id}"),
            Some(json!({ "name": "Platform" })),
        ),
        ("DELETE", format!("/v1/folders/{id}"), None),
        (
            "PUT",
            "/v1/dashboards/delivery/folder".to_owned(),
            Some(json!({ "folder": id })),
        ),
        ("GET", "/v1/dashboards?folder=unfiled".to_owned(), None),
    ];

    for (method, path, body) in routes {
        let refused = harness.send(method, &path, body).await;

        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{method} {path}");
    }
}

#[tokio::test]
async fn a_store_that_is_down_answers_a_server_error() {
    let harness = Harness::build(true, Arc::new(MemoryDefinitions::refusing()));

    let refused = harness.create("Platform").await;

    assert_eq!(refused.status, StatusCode::INTERNAL_SERVER_ERROR);
}
