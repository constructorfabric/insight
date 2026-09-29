use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use clickhouse::test::Mock;
use serde_json::{Value, json};
use toolkit::api::OpenApiRegistryImpl;
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::api::AppState;
use crate::chat::ChatClient;
use crate::domain::folders::DefinitionStore;
use crate::domain::pins::MAX_PINS;
use crate::domain::query::metric_query::{MetricRunner, People};
use crate::store::definitions::memory::MemoryDefinitions;
use crate::store::identity::IdentityClient;

const ANNA: Uuid = Uuid::from_u128(1);
const BORIS: Uuid = Uuid::from_u128(2);

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
        Self::build(
            IdentityClient::admin(ANNA),
            Arc::new(MemoryDefinitions::new()),
        )
    }

    fn build(identity: IdentityClient, store: Arc<dyn DefinitionStore>) -> Self {
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
            identity,
            crate::api::Datasets::holding(url, &[]),
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

    async fn pin(&self, name: &str) -> Answer {
        self.send("PUT", &format!("/v1/pins/{name}"), None).await
    }

    async fn pinned(&self, name: &str) {
        let answer = self.pin(name).await;
        assert_eq!(answer.status, StatusCode::NO_CONTENT, "{}", answer.body);
        assert_eq!(answer.body, Value::Null);
    }

    async fn unpin(&self, name: &str) -> Answer {
        self.send("DELETE", &format!("/v1/pins/{name}"), None).await
    }

    async fn pins(&self) -> Value {
        let answer = self.send("GET", "/v1/pins", None).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);

        answer.body
    }
}

#[tokio::test]
async fn a_caller_is_listed_the_pins_they_made_oldest_first() {
    let harness = Harness::new();
    for name in ["alpha", "beta", "gamma"] {
        harness.dashboard(name).await;
    }

    for name in ["gamma", "alpha", "beta"] {
        harness.pinned(name).await;
    }

    assert_eq!(
        harness.pins().await,
        json!({ "pins": ["gamma", "alpha", "beta"] })
    );
}

#[tokio::test]
async fn one_caller_s_pins_never_show_for_another() {
    let store: Arc<dyn DefinitionStore> = Arc::new(MemoryDefinitions::new());
    let anna = Harness::build(IdentityClient::admin(ANNA), Arc::clone(&store));
    let boris = Harness::build(IdentityClient::admin(BORIS), store);
    anna.dashboard("alpha").await;
    anna.dashboard("beta").await;

    anna.pinned("alpha").await;
    boris.pinned("beta").await;

    assert_eq!(anna.pins().await, json!({ "pins": ["alpha"] }));
    assert_eq!(boris.pins().await, json!({ "pins": ["beta"] }));
}

#[tokio::test]
async fn a_caller_with_no_pins_is_listed_none() {
    let harness = Harness::new();

    assert_eq!(harness.pins().await, json!({ "pins": [] }));
}

#[tokio::test]
async fn pinning_a_pinned_dashboard_again_keeps_its_place() {
    let harness = Harness::new();
    harness.dashboard("alpha").await;
    harness.dashboard("beta").await;
    harness.pinned("alpha").await;
    harness.pinned("beta").await;

    harness.pinned("alpha").await;

    assert_eq!(harness.pins().await, json!({ "pins": ["alpha", "beta"] }));
}

#[tokio::test]
async fn an_unpin_answers_no_content_whether_or_not_it_was_pinned() {
    let harness = Harness::new();
    harness.dashboard("alpha").await;
    harness.dashboard("beta").await;
    harness.pinned("alpha").await;

    let unpinned = harness.unpin("alpha").await;
    let never_pinned = harness.unpin("beta").await;

    for answer in [unpinned, never_pinned] {
        assert_eq!(answer.status, StatusCode::NO_CONTENT, "{}", answer.body);
        assert_eq!(answer.body, Value::Null);
    }
    assert_eq!(harness.pins().await, json!({ "pins": [] }));
}

#[tokio::test]
async fn pinning_or_unpinning_a_dashboard_that_is_not_there_is_not_found() {
    let harness = Harness::new();

    for refused in [harness.pin("nowhere").await, harness.unpin("nowhere").await] {
        assert_eq!(refused.status, StatusCode::NOT_FOUND, "{}", refused.body);
        assert!(
            refused.body.to_string().contains("`nowhere`"),
            "{}",
            refused.body
        );
    }
}

#[tokio::test]
async fn a_pin_past_the_cap_is_a_conflict_that_says_why() {
    let harness = Harness::new();
    for index in 0..=MAX_PINS {
        harness.dashboard(&format!("b{index}")).await;
    }
    for index in 0..MAX_PINS {
        harness.pinned(&format!("b{index}")).await;
    }

    let refused = harness.pin(&format!("b{MAX_PINS}")).await;

    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(
        refused.body.to_string().contains("at most 20 dashboards"),
        "{}",
        refused.body
    );
    harness.pinned("b0").await;
}

#[tokio::test]
async fn a_name_no_dashboard_could_have_is_refused() {
    let harness = Harness::new();

    for refused in [
        harness.pin("not%20a%20name").await,
        harness.unpin("not%20a%20name").await,
    ] {
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    }
}

#[tokio::test]
async fn a_renamed_dashboard_is_listed_under_its_new_name_in_its_place() {
    let harness = Harness::new();
    for name in ["alpha", "beta", "gamma"] {
        harness.dashboard(name).await;
        harness.pinned(name).await;
    }

    let renamed = harness
        .send(
            "POST",
            "/v1/dashboards/beta/rename",
            Some(json!({ "to": "delta" })),
        )
        .await;

    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    assert_eq!(
        harness.pins().await,
        json!({ "pins": ["alpha", "delta", "gamma"] })
    );
}

#[tokio::test]
async fn a_deleted_dashboard_leaves_the_pinned_list() {
    let harness = Harness::new();
    harness.dashboard("alpha").await;
    harness.dashboard("beta").await;
    harness.pinned("alpha").await;
    harness.pinned("beta").await;

    let removed = harness.send("DELETE", "/v1/dashboards/alpha", None).await;

    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(harness.pins().await, json!({ "pins": ["beta"] }));
}

#[tokio::test]
async fn a_caller_without_the_admin_role_is_refused_on_every_pin_route() {
    let harness = Harness::build(
        IdentityClient::fixed(false),
        Arc::new(MemoryDefinitions::new()),
    );

    for (method, path) in [
        ("GET", "/v1/pins"),
        ("PUT", "/v1/pins/alpha"),
        ("DELETE", "/v1/pins/alpha"),
    ] {
        let refused = harness.send(method, path, None).await;

        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{method} {path}");
    }
}

#[tokio::test]
async fn a_store_that_is_down_answers_a_server_error() {
    let harness = Harness::build(
        IdentityClient::admin(ANNA),
        Arc::new(MemoryDefinitions::refusing()),
    );

    let refused = harness.pin("alpha").await;

    assert_eq!(refused.status, StatusCode::INTERNAL_SERVER_ERROR);
}
