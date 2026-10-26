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
use crate::domain::access::PersonId;
use crate::domain::definition::{DefinitionId, DefinitionKind};
use crate::domain::query::metric_query::{MetricRunner, People};
use crate::domain::tags::MAX_TAGS;
use crate::store::definitions::memory::MemoryDefinitions;

/// The administrator the fixed identity answers for a request naming none.
const ME: &str = "00000000-0000-0000-0000-000000000000";

const SOMEONE_ELSE: &str = "0192a1b2-0000-7000-8000-000000000002";

fn person(raw: &str) -> PersonId {
    PersonId::parse(raw).unwrap_or_else(|| panic!("`{raw}` is a person id"))
}

fn commits() -> Value {
    json!({
        "title": "Commits",
        "fields": [{ "name": "day", "path": "day", "type": "string" }]
    })
}

struct Harness {
    _clickhouse: Mock,
    router: Router,
    definitions: Arc<MemoryDefinitions>,
}

/// The ids a listing answers, in its order.
fn ids(listed: &Value) -> Value {
    let Some(items) = listed["items"].as_array() else {
        panic!("a listing answers items: {listed}");
    };

    items.iter().map(|item| item["id"].clone()).collect()
}

struct Answer {
    status: StatusCode,
    body: Value,
}

impl Harness {
    fn new() -> Self {
        Self::build(true, Arc::new(MemoryDefinitions::new()))
    }

    fn build(is_admin: bool, store: Arc<MemoryDefinitions>) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let url = mock.url();
        let openapi = OpenApiRegistryImpl::new();
        let state = Arc::new(AppState::new(
            MetricRunner::new(
                insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "insight")),
                People::new("identity"),
            ),
            store.clone(),
            ChatClient::keyless(),
            crate::store::identity::IdentityClient::fixed(is_admin),
            crate::api::Datasets::holding(url, &[("commits", commits())]),
            crate::store::catalog::Catalog::fixed(Vec::new()),
        ));

        let router = crate::api::definitions::register_routes(Router::new(), &openapi, &state);
        let router = crate::api::folders::register_routes(router, &openapi, &state);
        let router = super::register_routes(router, &openapi, &state);

        Self {
            _clickhouse: mock,
            router,
            definitions: store,
        }
    }

    async fn send(&self, method: &str, path: &str, body: Option<Value>) -> Answer {
        self.send_as(ME, method, path, body).await
    }

    /// A request from the administrator `caller`.
    async fn send_as(&self, caller: &str, method: &str, path: &str, body: Option<Value>) -> Answer {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {caller}"));
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

    /// Stores a dashboard of mine under an id the case chooses, or writes
    /// its body again through the route when it is already there.
    async fn dashboard(&self, id: &str) {
        let path = format!("/v1/dashboards/{id}");
        let body = json!({ "title": id, "widgets": [] });
        if self.send("GET", &path, None).await.status == StatusCode::NOT_FOUND {
            self.owned_by(id, ME).await;
            return;
        }

        let answer = self
            .send("PUT", &path, Some(json!({ "name": id, "body": body })))
            .await;
        assert_eq!(answer.status, StatusCode::NO_CONTENT, "{}", answer.body);
    }

    async fn owned_by(&self, id: &str, owner: &str) {
        let parsed = DefinitionId::parse(id).unwrap_or_else(|error| panic!("{id}: {error}"));
        self.definitions
            .seed_owned(
                &parsed,
                &json!({ "title": id, "widgets": [] }),
                person(owner),
            )
            .await
            .unwrap_or_else(|error| panic!("`{id}` must store: {error}"));
    }

    async fn shared_with(&self, id: &str, viewer: &str, owner: &str) {
        let parsed = DefinitionId::parse(id).unwrap_or_else(|error| panic!("{id}: {error}"));
        self.definitions
            .seed_viewer(&parsed, person(viewer), person(owner))
            .await
            .unwrap_or_else(|error| panic!("`{id}` must share: {error}"));
    }

    async fn tag(&self, dashboard: &str, tags: Value) -> Answer {
        self.send(
            "PUT",
            &format!("/v1/dashboards/{dashboard}/tags"),
            Some(json!({ "tags": tags })),
        )
        .await
    }

    async fn tagged(&self, dashboard: &str, tags: Value) {
        let answer = self.tag(dashboard, tags).await;
        assert_eq!(answer.status, StatusCode::NO_CONTENT, "{}", answer.body);
    }

    async fn read(&self, dashboard: &str) -> Value {
        let answer = self
            .send("GET", &format!("/v1/dashboards/{dashboard}"), None)
            .await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);

        answer.body
    }

    async fn tags(&self) -> Value {
        let answer = self.send("GET", "/v1/tags", None).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);

        answer.body
    }

    async fn listed(&self, query: &str) -> Answer {
        self.send("GET", &format!("/v1/dashboards?{query}"), None)
            .await
    }
}

#[tokio::test]
async fn a_tag_is_listed_with_how_many_dashboards_carry_it() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;
    harness.dashboard("support").await;

    harness
        .tagged("delivery", json!(["Platform", "Delivery"]))
        .await;
    harness.tagged("support", json!(["Platform"])).await;

    assert_eq!(
        harness.tags().await,
        json!({
            "tags": [
                { "name": "Delivery", "dashboards": 1 },
                { "name": "Platform", "dashboards": 2 }
            ]
        })
    );
}

#[tokio::test]
async fn an_existing_tag_is_reused_under_its_stored_spelling() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;
    harness.dashboard("support").await;
    harness.tagged("delivery", json!(["Delivery"])).await;

    harness.tagged("support", json!(["  delivery "])).await;

    assert_eq!(harness.read("support").await["tags"], json!(["Delivery"]));
    assert_eq!(
        harness.tags().await,
        json!({ "tags": [{ "name": "Delivery", "dashboards": 2 }] })
    );
}

#[tokio::test]
async fn a_dashboard_read_names_its_tags_sorted_beside_its_folder() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;
    harness.dashboard("hiring").await;

    harness
        .tagged("delivery", json!(["beta", "Alpha", "gamma"]))
        .await;
    let tagged = harness.read("delivery").await;
    let untagged = harness.read("hiring").await;

    assert_eq!(tagged["tags"], json!(["Alpha", "beta", "gamma"]));
    assert_eq!(tagged["folder"], Value::Null);
    assert_eq!(untagged["tags"], json!([]));
}

#[tokio::test]
async fn an_empty_set_clears_and_a_tag_nobody_carries_is_gone() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;
    harness.tagged("delivery", json!(["Delivery"])).await;

    harness.tagged("delivery", json!([])).await;

    assert_eq!(harness.read("delivery").await["tags"], json!([]));
    assert_eq!(harness.tags().await, json!({ "tags": [] }));
}

#[tokio::test]
async fn a_blank_or_overlong_name_is_refused_and_says_why() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;

    for name in ["   ".to_owned(), "x".repeat(33)] {
        let refused = harness.tag("delivery", json!([name])).await;

        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{name:?}");
        assert!(
            refused.body.to_string().contains("1 to 32 characters"),
            "{}",
            refused.body
        );
    }
    harness.tagged("delivery", json!(["x".repeat(32)])).await;
}

#[tokio::test]
async fn more_than_ten_distinct_tags_are_refused_and_say_why() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;
    let eleven: Vec<String> = (0..11).map(|index| format!("t{index}")).collect();
    let mut ten_in_eleven_spellings: Vec<String> = eleven[..10].to_vec();
    ten_in_eleven_spellings.push("T0".to_owned());

    let refused = harness.tag("delivery", json!(eleven)).await;
    let collapsed = harness
        .tag("delivery", json!(ten_in_eleven_spellings))
        .await;

    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(
        refused.body.to_string().contains("at most 10 tags"),
        "{}",
        refused.body
    );
    assert_eq!(
        collapsed.status,
        StatusCode::NO_CONTENT,
        "{}",
        collapsed.body
    );
}

#[tokio::test]
async fn a_body_without_its_tags_is_refused() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;

    for body in [
        json!({}),
        json!({ "tags": null }),
        json!({ "tags": "Delivery" }),
    ] {
        let refused = harness
            .send("PUT", "/v1/dashboards/delivery/tags", Some(body.clone()))
            .await;

        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{body}");
    }
}

#[tokio::test]
async fn tagging_a_dashboard_that_is_not_there_is_not_found() {
    let harness = Harness::new();

    let refused = harness.tag("nowhere", json!(["Delivery"])).await;

    assert_eq!(refused.status, StatusCode::NOT_FOUND, "{}", refused.body);
    assert!(
        refused.body.to_string().contains("`nowhere`"),
        "{}",
        refused.body
    );
}

#[tokio::test]
async fn a_set_past_the_cap_is_a_conflict_that_says_why() {
    let harness = Harness::new();
    for board in 0..MAX_TAGS / 10 {
        let name = format!("b{board}");
        harness.dashboard(&name).await;
        let full: Vec<String> = (0..10).map(|tag| format!("t{board}_{tag}")).collect();
        harness.tagged(&name, json!(full)).await;
    }
    harness.dashboard("last").await;

    let refused = harness.tag("last", json!(["one more"])).await;

    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(
        refused.body.to_string().contains("200 tags"),
        "{}",
        refused.body
    );
    harness.tagged("last", json!(["t0_0"])).await;
}

#[tokio::test]
async fn a_rewritten_dashboard_keeps_its_tags() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;
    harness.tagged("delivery", json!(["Delivery"])).await;

    harness.dashboard("delivery").await;

    assert_eq!(harness.read("delivery").await["tags"], json!(["Delivery"]));
}

#[tokio::test]
async fn tagging_an_id_that_cannot_be_one_is_not_found() {
    let harness = Harness::new();

    let refused = harness.tag("not%20an%20id", json!(["Delivery"])).await;

    assert_eq!(refused.status, StatusCode::NOT_FOUND, "{}", refused.body);
    assert_eq!(harness.tags().await, json!({ "tags": [] }));
}

#[tokio::test]
async fn a_deleted_dashboard_takes_the_tags_only_it_carried() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;
    harness.dashboard("support").await;
    harness
        .tagged("delivery", json!(["Delivery", "Shared"]))
        .await;
    harness.tagged("support", json!(["Shared"])).await;

    let removed = harness
        .send("DELETE", "/v1/dashboards/delivery", None)
        .await;

    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(
        harness.tags().await,
        json!({ "tags": [{ "name": "Shared", "dashboards": 1 }] })
    );
}

#[tokio::test]
async fn a_dashboard_list_narrows_to_those_carrying_any_of_the_tags() {
    let harness = Harness::new();
    for (name, tags) in [
        ("delivery", json!(["Alpha"])),
        ("delivery_ai", json!(["Alpha", "Beta team"])),
        ("hiring", json!(["Beta team"])),
        ("support", json!(["Gamma"])),
    ] {
        harness.dashboard(name).await;
        harness.tagged(name, tags).await;
    }

    let listed = harness.listed("tag=alpha&tag=BETA%20team").await;

    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert_eq!(
        ids(&listed.body),
        json!(["delivery", "delivery_ai", "hiring"])
    );
    assert_eq!(listed.body["total"], 3);
}

#[tokio::test]
async fn a_list_by_tag_keeps_to_its_folder_and_its_search() {
    let harness = Harness::new();
    for name in ["delivery", "delivery_ai", "hiring"] {
        harness.dashboard(name).await;
        harness.tagged(name, json!(["Alpha"])).await;
    }
    let created = harness
        .send("POST", "/v1/folders", Some(json!({ "name": "Platform" })))
        .await;
    let folder = created.body["id"].as_str().unwrap_or_default().to_owned();
    for name in ["delivery", "hiring"] {
        harness
            .send(
                "PUT",
                &format!("/v1/folders/{folder}/dashboards/{name}"),
                None,
            )
            .await;
    }

    let filed = harness.listed(&format!("tag=Alpha&folder={folder}")).await;
    let unfiled = harness.listed("tag=Alpha&folder=unfiled").await;
    let searched = harness.listed("tag=Alpha&q=deliv").await;
    let both = harness
        .listed(&format!("tag=Alpha&folder={folder}&q=hir"))
        .await;

    assert_eq!(ids(&filed.body), json!(["delivery", "hiring"]));
    assert_eq!(ids(&unfiled.body), json!(["delivery_ai"]));
    assert_eq!(ids(&searched.body), json!(["delivery", "delivery_ai"]));
    assert_eq!(ids(&both.body), json!(["hiring"]));
}

#[tokio::test]
async fn a_list_by_a_tag_nobody_carries_is_empty() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;
    harness.tagged("delivery", json!(["Alpha"])).await;

    let listed = harness.listed("tag=Omega").await;

    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(ids(&listed.body), json!([]));
    assert_eq!(listed.body["total"], 0);
}

#[tokio::test]
async fn only_dashboards_are_listed_by_tag() {
    let harness = Harness::new();

    for kind in ["metrics", "widgets"] {
        let refused = harness
            .send("GET", &format!("/v1/{kind}?tag=Alpha"), None)
            .await;

        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{kind}");
    }
}

#[tokio::test]
async fn a_list_by_a_blank_tag_is_refused() {
    let harness = Harness::new();

    let refused = harness.listed("tag=").await;

    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_read_of_another_kind_names_no_tags() {
    let harness = Harness::new();
    let made = harness
        .send(
            "POST",
            "/v1/metrics",
            Some(json!({
                "name": "Commit count",
                "body": {
                    "dataset": "commits",
                    "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
                }
            })),
        )
        .await;
    let id = made.body["id"].as_str().unwrap_or_default();

    let read = harness
        .send("GET", &format!("/v1/metrics/{id}"), None)
        .await;

    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert!(read.body.get("tags").is_none(), "{}", read.body);
}

#[tokio::test]
async fn a_member_lists_tags_but_is_refused_setting_them() {
    let harness = Harness::build(false, Arc::new(MemoryDefinitions::new()));

    let listed = harness.send("GET", "/v1/tags", None).await;
    let by_tag = harness.send("GET", "/v1/dashboards?tag=Alpha", None).await;
    let refused = harness
        .send(
            "PUT",
            "/v1/dashboards/delivery/tags",
            Some(json!({ "tags": ["Alpha"] })),
        )
        .await;

    assert_eq!(listed.body, json!({ "tags": [] }));
    assert_eq!(by_tag.body["total"], 0, "{}", by_tag.body);
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
}

#[tokio::test]
async fn a_store_that_is_down_answers_a_server_error() {
    let harness = Harness::build(true, Arc::new(MemoryDefinitions::refusing()));

    let refused = harness.tag("delivery", json!(["Alpha"])).await;

    assert_eq!(refused.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn two_owners_tags_of_one_name_coexist_and_a_filter_finds_both() {
    let harness = Harness::new();
    harness.dashboard("mine").await;
    harness.owned_by("theirs", SOMEONE_ELSE).await;
    harness.shared_with("theirs", ME, SOMEONE_ELSE).await;
    harness.tagged("mine", json!(["Delivery"])).await;
    let theirs = harness
        .send_as(
            SOMEONE_ELSE,
            "PUT",
            "/v1/dashboards/theirs/tags",
            Some(json!({ "tags": ["delivery"] })),
        )
        .await;

    let listed = harness.listed("tag=DELIVERY").await;

    assert_eq!(theirs.status, StatusCode::NO_CONTENT, "{}", theirs.body);
    assert_eq!(harness.read("theirs").await["tags"], json!(["delivery"]));
    assert_eq!(ids(&listed.body), json!(["mine", "theirs"]));
    assert_eq!(
        harness.tags().await,
        json!({ "tags": [{ "name": "Delivery", "dashboards": 2 }] })
    );
}

#[tokio::test]
async fn tags_count_only_the_dashboards_the_caller_sees() {
    let harness = Harness::new();
    harness.dashboard("mine").await;
    harness.owned_by("private", SOMEONE_ELSE).await;
    harness.tagged("mine", json!(["Shared"])).await;
    harness
        .send_as(
            SOMEONE_ELSE,
            "PUT",
            "/v1/dashboards/private/tags",
            Some(json!({ "tags": ["Shared", "Secret"] })),
        )
        .await;

    assert_eq!(
        harness.tags().await,
        json!({ "tags": [{ "name": "Shared", "dashboards": 1 }] })
    );
    assert_eq!(ids(&harness.listed("tag=Secret").await.body), json!([]));
}

#[tokio::test]
async fn a_viewer_may_not_tag_and_someone_who_cannot_see_it_finds_nothing() {
    let harness = Harness::new();
    harness.owned_by("theirs", SOMEONE_ELSE).await;
    harness.shared_with("theirs", ME, SOMEONE_ELSE).await;
    harness.owned_by("private", SOMEONE_ELSE).await;

    let as_viewer = harness.tag("theirs", json!(["Alpha"])).await;
    let unseen = harness.tag("private", json!(["Alpha"])).await;
    let missing = harness.tag("nowhere", json!(["Alpha"])).await;

    assert_eq!(
        as_viewer.status,
        StatusCode::FORBIDDEN,
        "{}",
        as_viewer.body
    );
    assert!(
        as_viewer
            .body
            .to_string()
            .contains("only the owner or an editor can change this dashboard"),
        "{}",
        as_viewer.body
    );
    assert_eq!(unseen.status, StatusCode::NOT_FOUND, "{}", unseen.body);
    assert_eq!(
        unseen.body.to_string().replace("private", "nowhere"),
        missing.body.to_string(),
        "a dashboard the caller cannot see answers as a missing one"
    );
}

#[tokio::test]
async fn an_ownerless_dashboard_is_tagged_by_anyone() {
    let harness = Harness::new();
    let parsed = DefinitionId::parse("legacy").unwrap_or_else(|error| panic!("{error}"));
    harness
        .definitions
        .seed(
            DefinitionKind::Dashboard,
            &parsed,
            &json!({ "title": "legacy", "widgets": [] }),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let tagged = harness
        .send_as(
            SOMEONE_ELSE,
            "PUT",
            "/v1/dashboards/legacy/tags",
            Some(json!({ "tags": ["Legacy"] })),
        )
        .await;

    assert_eq!(tagged.status, StatusCode::NO_CONTENT, "{}", tagged.body);
    assert_eq!(harness.read("legacy").await["tags"], json!(["Legacy"]));
}
