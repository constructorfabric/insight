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
use crate::domain::query::metric_query::{MetricRunner, People};
use crate::store::definitions::memory::MemoryDefinitions;
use crate::store::identity::IdentityClient;

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
        Self::build(IdentityClient::fixed(true))
    }

    fn build(identity: IdentityClient) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let url = mock.url();
        let openapi = OpenApiRegistryImpl::new();
        let state = Arc::new(AppState::new(
            MetricRunner::new(
                insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "insight")),
                People::new("identity"),
            ),
            Arc::new(MemoryDefinitions::new()),
            ChatClient::keyless(),
            identity,
            crate::api::Datasets::holding(url, &[]),
            crate::store::catalog::Catalog::fixed(Vec::new()),
        ));

        let router = crate::api::definitions::register_routes(Router::new(), &openapi, &state);
        let router = crate::api::folders::register_routes(router, &openapi, &state);
        let router = crate::api::tags::register_routes(router, &openapi, &state);
        let router = crate::api::pins::register_routes(router, &openapi, &state);
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

    async fn sent(&self, method: &str, path: &str, body: Option<Value>) -> Value {
        let answer = self.send(method, path, body).await;
        assert!(
            answer.status.is_success(),
            "{method} {path}: {} {}",
            answer.status,
            answer.body
        );

        answer.body
    }

    /// A dashboard filed in a folder, tagged and pinned, by the id it was
    /// given.
    async fn filed_tagged_and_pinned(&self) -> String {
        let made = self
            .sent(
                "POST",
                "/v1/dashboards",
                Some(json!({
                    "name": "Delivery",
                    "body": { "title": "Delivery", "widgets": [] }
                })),
            )
            .await;
        let id = made["id"]
            .as_str()
            .unwrap_or_else(|| panic!("a create answers its id: {made}"))
            .to_owned();
        let folder = self
            .sent("POST", "/v1/folders", Some(json!({ "name": "Platform" })))
            .await;
        let folder = folder["id"]
            .as_str()
            .unwrap_or_else(|| panic!("a folder answers its id: {folder}"));
        self.sent(
            "PUT",
            &format!("/v1/folders/{folder}/dashboards/{id}"),
            None,
        )
        .await;
        self.sent(
            "PUT",
            &format!("/v1/dashboards/{id}/tags"),
            Some(json!({ "tags": ["Delivery", "Platform"] })),
        )
        .await;
        self.sent("PUT", &format!("/v1/pins/{id}"), None).await;

        id
    }

    async fn duplicate(&self, from: &str, body: Value) -> Answer {
        self.send(
            "POST",
            &format!("/v1/dashboards/{from}/duplicate"),
            Some(body),
        )
        .await
    }
}

fn answered_id(answer: &Answer) -> String {
    answer.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a copy answers its id: {}", answer.body))
        .to_owned()
}

#[tokio::test]
async fn a_duplicate_is_created_under_a_fresh_id_with_the_body_folder_and_tags() {
    let harness = Harness::new();
    let source_id = harness.filed_tagged_and_pinned().await;
    let source_path = format!("/v1/dashboards/{source_id}");
    let source = harness.sent("GET", &source_path, None).await;

    let created = harness
        .duplicate(&source_id, json!({ "name": "Delivery copy" }))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let copy_id = answered_id(&created);
    let copy = harness
        .sent("GET", &format!("/v1/dashboards/{copy_id}"), None)
        .await;

    assert_ne!(copy_id, source_id);
    assert_eq!(copy["name"], "Delivery copy");
    for field in ["body", "folders", "tags"] {
        assert_eq!(copy[field], source[field], "{field}");
    }
    assert_eq!(harness.sent("GET", &source_path, None).await, source);
}

#[tokio::test]
async fn a_duplicate_is_not_pinned() {
    let harness = Harness::new();
    let source_id = harness.filed_tagged_and_pinned().await;

    let created = harness
        .duplicate(&source_id, json!({ "name": "Delivery copy" }))
        .await;

    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(
        harness.sent("GET", "/v1/pins", None).await,
        json!({ "pins": [{ "id": source_id, "name": "Delivery" }] })
    );
}

#[tokio::test]
async fn a_duplicate_may_share_its_source_s_name() {
    let harness = Harness::new();
    let source_id = harness.filed_tagged_and_pinned().await;

    let created = harness
        .duplicate(&source_id, json!({ "name": "Delivery" }))
        .await;

    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let listed = harness.sent("GET", "/v1/dashboards", None).await;
    assert_eq!(listed["total"], json!(2));
}

#[tokio::test]
async fn a_duplicate_of_a_dashboard_that_is_not_there_is_not_found() {
    let harness = Harness::new();

    let refused = harness
        .duplicate("nowhere", json!({ "name": "Nowhere copy" }))
        .await;
    let malformed = harness
        .duplicate("not%20an%20id", json!({ "name": "Nowhere copy" }))
        .await;

    for answer in [&refused, &malformed] {
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.body);
    }
    assert!(
        refused.body.to_string().contains("`nowhere`"),
        "{}",
        refused.body
    );
    let listed = harness.sent("GET", "/v1/dashboards", None).await;
    assert_eq!(listed["total"], json!(0));
}

#[tokio::test]
async fn a_new_name_that_is_missing_or_not_a_name_is_refused() {
    let harness = Harness::new();
    let source_id = harness.filed_tagged_and_pinned().await;

    for body in [
        json!({}),
        json!({ "name": null }),
        json!({ "name": 5 }),
        json!({ "name": "" }),
        json!({ "name": "   " }),
        json!({ "name": "x".repeat(129) }),
    ] {
        let refused = harness.duplicate(&source_id, body.clone()).await;

        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{body}");
    }
    let listed = harness.sent("GET", "/v1/dashboards", None).await;
    assert_eq!(listed["total"], json!(1));
}

#[tokio::test]
async fn a_caller_without_the_admin_role_is_refused() {
    let harness = Harness::build(IdentityClient::fixed(false));

    for body in [json!({ "name": "Delivery copy" }), json!({})] {
        let refused = harness.duplicate("delivery", body.clone()).await;

        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{body}");
    }
}
