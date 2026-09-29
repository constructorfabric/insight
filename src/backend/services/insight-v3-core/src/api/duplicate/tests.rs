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

    async fn filed_tagged_and_pinned(&self, name: &str) {
        self.sent(
            "PUT",
            &format!("/v1/dashboards/{name}"),
            Some(json!({ "title": "Delivery", "widgets": [] })),
        )
        .await;
        let folder = self
            .sent("POST", "/v1/folders", Some(json!({ "name": "Platform" })))
            .await;
        self.sent(
            "PUT",
            &format!("/v1/dashboards/{name}/folder"),
            Some(json!({ "folder": folder["id"] })),
        )
        .await;
        self.sent(
            "PUT",
            &format!("/v1/dashboards/{name}/tags"),
            Some(json!({ "tags": ["Delivery", "Platform"] })),
        )
        .await;
        self.sent("PUT", &format!("/v1/pins/{name}"), None).await;
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

#[tokio::test]
async fn a_duplicate_is_created_under_its_new_name_with_the_body_folder_and_tags() {
    let harness = Harness::new();
    harness.filed_tagged_and_pinned("delivery").await;
    let source = harness.sent("GET", "/v1/dashboards/delivery", None).await;

    let created = harness
        .duplicate("delivery", json!({ "name": "delivery-copy" }))
        .await;
    let copy = harness
        .sent("GET", "/v1/dashboards/delivery-copy", None)
        .await;

    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.body, json!({ "name": "delivery-copy" }));
    for field in ["body", "folder", "tags"] {
        assert_eq!(copy[field], source[field], "{field}");
    }
    assert_eq!(
        harness.sent("GET", "/v1/dashboards/delivery", None).await,
        source
    );
}

#[tokio::test]
async fn a_duplicate_is_not_pinned() {
    let harness = Harness::new();
    harness.filed_tagged_and_pinned("delivery").await;

    let created = harness
        .duplicate("delivery", json!({ "name": "delivery-copy" }))
        .await;

    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(
        harness.sent("GET", "/v1/pins", None).await,
        json!({ "pins": ["delivery"] })
    );
}

#[tokio::test]
async fn a_duplicate_onto_a_name_already_held_is_a_conflict_that_names_it() {
    let harness = Harness::new();
    harness.filed_tagged_and_pinned("delivery").await;
    harness
        .sent(
            "PUT",
            "/v1/dashboards/hiring",
            Some(json!({ "title": "Hiring", "widgets": [] })),
        )
        .await;

    for onto in ["hiring", "delivery"] {
        let refused = harness.duplicate("delivery", json!({ "name": onto })).await;

        assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
        assert!(
            refused.body.to_string().contains(&format!("`{onto}`")),
            "{}",
            refused.body
        );
    }
    let hiring = harness.sent("GET", "/v1/dashboards/hiring", None).await;
    assert_eq!(hiring["body"]["title"], "Hiring");
    assert_eq!(hiring["folder"], Value::Null);
    assert_eq!(hiring["tags"], json!([]));
}

#[tokio::test]
async fn a_duplicate_of_a_dashboard_that_is_not_there_is_not_found() {
    let harness = Harness::new();

    let refused = harness
        .duplicate("nowhere", json!({ "name": "nowhere-copy" }))
        .await;

    assert_eq!(refused.status, StatusCode::NOT_FOUND, "{}", refused.body);
    assert!(
        refused.body.to_string().contains("`nowhere`"),
        "{}",
        refused.body
    );
    assert_eq!(
        harness
            .send("GET", "/v1/dashboards/nowhere-copy", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_new_name_that_is_missing_or_not_a_name_is_refused() {
    let harness = Harness::new();
    harness.filed_tagged_and_pinned("delivery").await;

    for body in [
        json!({}),
        json!({ "name": null }),
        json!({ "name": 5 }),
        json!({ "name": "" }),
        json!({ "name": "not a name" }),
        json!({ "name": "x".repeat(129) }),
    ] {
        let refused = harness.duplicate("delivery", body.clone()).await;

        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{body}");
    }
    assert_eq!(
        harness.sent("GET", "/v1/dashboards", None).await["names"],
        json!(["delivery"])
    );
}

#[tokio::test]
async fn a_caller_without_the_admin_role_is_refused() {
    let harness = Harness::build(IdentityClient::fixed(false));

    for body in [json!({ "name": "delivery-copy" }), json!({})] {
        let refused = harness.duplicate("delivery", body.clone()).await;

        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{body}");
    }
}
