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
use crate::store::definitions::memory::MemoryDefinitions;

/// The administrator the fixed identity answers for a request naming none.
const ME: &str = "00000000-0000-0000-0000-000000000000";

const SOMEONE_ELSE: &str = "0192a1b2-0000-7000-8000-000000000002";

fn me() -> PersonId {
    PersonId::new(uuid::Uuid::nil())
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

struct Answer {
    status: StatusCode,
    body: Value,
}

fn listed_ids(listed: &Answer) -> Vec<&str> {
    listed.body["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["id"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

impl Harness {
    fn new() -> Self {
        Self::build(true, Arc::new(MemoryDefinitions::new()))
    }

    fn build(is_admin: bool, store: Arc<MemoryDefinitions>) -> Self {
        let identity = crate::store::identity::IdentityClient::fixed(is_admin);

        Self::with_identity(identity, store)
    }

    fn with_identity(
        identity: crate::store::identity::IdentityClient,
        store: Arc<MemoryDefinitions>,
    ) -> Self {
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
            identity.with_administrators(&[
                uuid::Uuid::nil(),
                uuid::Uuid::from_u128(0x0192_a1b2_0000_7000_8000_0000_0000_0002),
            ]),
            crate::api::Datasets::holding(url, &[("commits", commits())]),
            crate::store::catalog::Catalog::fixed(Vec::new()),
        ));

        let router = crate::api::definitions::register_routes(Router::new(), &openapi, &state);
        let router = crate::api::sharing::register_routes(router, &openapi, &state);
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

    /// A request from the administrator `person`.
    async fn send_as(&self, person: &str, method: &str, path: &str, body: Option<Value>) -> Answer {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {person}"));
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

    /// Stores a dashboard of mine under an id the case chooses, or writes
    /// its body again through the route when it is already there.
    async fn dashboard(&self, id: &str) {
        let path = format!("/v1/dashboards/{id}");
        let body = json!({ "title": id, "widgets": [] });
        if self.send("GET", &path, None).await.status == StatusCode::NOT_FOUND {
            let parsed = DefinitionId::parse(id).unwrap_or_else(|error| panic!("{id}: {error}"));
            self.definitions
                .seed_owned(&parsed, &body, me())
                .await
                .unwrap_or_else(|error| panic!("`{id}` must store: {error}"));
            return;
        }

        let answer = self
            .send("PUT", &path, Some(json!({ "name": id, "body": body })))
            .await;
        assert_eq!(answer.status, StatusCode::NO_CONTENT, "{}", answer.body);
    }

    /// Stores a dashboard predating ownership under an id the case chooses.
    async fn ownerless(&self, id: &str) {
        let parsed = DefinitionId::parse(id).unwrap_or_else(|error| panic!("{id}: {error}"));
        self.definitions
            .seed(
                DefinitionKind::Dashboard,
                &parsed,
                &json!({ "title": id, "widgets": [] }),
            )
            .await
            .unwrap_or_else(|error| panic!("`{id}` must store: {error}"));
    }

    async fn file(&self, dashboard: &str, folder: &str) -> Answer {
        self.send(
            "PUT",
            &format!("/v1/folders/{folder}/dashboards/{dashboard}"),
            None,
        )
        .await
    }

    async fn unfile(&self, dashboard: &str, folder: &str) -> Answer {
        self.send(
            "DELETE",
            &format!("/v1/folders/{folder}/dashboards/{dashboard}"),
            None,
        )
        .await
    }

    /// The folders the caller sees holding the dashboard.
    async fn folders_of(&self, dashboard: &str) -> Value {
        let answer = self
            .send("GET", &format!("/v1/dashboards/{dashboard}"), None)
            .await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);

        answer.body["folders"].clone()
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
            "folders": [{
                "id": created.body["id"],
                "name": "Platform",
                "owner_person_id": ME,
                "access": "owner",
                "dashboards": 0
            }],
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
async fn two_folders_may_share_a_name() {
    let harness = Harness::new();
    let first = harness.created_id("Platform").await;

    let second = harness.created_id("platform").await;

    assert_ne!(first, second);
    let listed = harness.send("GET", "/v1/folders", None).await;
    assert_eq!(listed.body["folders"].as_array().map(Vec::len), Some(2));
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
    harness.file("delivery", &id).await;

    let renamed = harness
        .send(
            "PATCH",
            &format!("/v1/folders/{id}"),
            Some(json!({ "name": "Platform team" })),
        )
        .await;

    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    assert_eq!(
        renamed.body,
        json!({ "id": id, "name": "Platform team", "owner_person_id": ME, "access": "owner" })
    );
    assert_eq!(
        harness.folders_of("delivery").await,
        json!([{ "id": id, "name": "Platform team" }])
    );
}

#[tokio::test]
async fn renaming_onto_another_folder_s_name_is_allowed() {
    let harness = Harness::new();
    harness.created_id("Platform").await;
    let product = harness.created_id("Product").await;

    let renamed = harness
        .send(
            "PATCH",
            &format!("/v1/folders/{product}"),
            Some(json!({ "name": "PLATFORM" })),
        )
        .await;

    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
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
async fn a_folder_id_that_is_not_a_uuid_names_no_folder() {
    let harness = Harness::new();

    let renamed = harness
        .send(
            "PATCH",
            "/v1/folders/platform",
            Some(json!({ "name": "Platform" })),
        )
        .await;
    let removed = harness.send("DELETE", "/v1/folders/platform", None).await;

    assert_eq!(renamed.status, StatusCode::NOT_FOUND);
    assert_eq!(removed.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_a_folder_leaves_its_dashboards_unfiled() {
    let harness = Harness::new();
    let id = harness.created_id("Platform").await;
    harness.dashboard("delivery").await;
    harness.file("delivery", &id).await;

    let removed = harness
        .send("DELETE", &format!("/v1/folders/{id}"), None)
        .await;

    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(harness.folders_of("delivery").await, json!([]));
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

    let moved = harness.file("delivery", &id).await;
    let read = harness.send("GET", "/v1/dashboards/delivery", None).await;

    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    assert_eq!(moved.body, json!({ "carries": true }));
    assert_eq!(read.body["carries"], json!(true));
    assert_eq!(
        read.body["folders"],
        json!([{ "id": id, "name": "Platform" }])
    );
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
    harness.file("delivery", &id).await;

    harness.dashboard("delivery").await;

    assert_eq!(harness.folders_of("delivery").await[0]["id"], json!(id));
}

#[tokio::test]
async fn a_dashboard_taken_out_of_its_only_folder_is_unfiled_and_taking_it_out_again_changes_nothing()
 {
    let harness = Harness::new();
    let id = harness.created_id("Platform").await;
    harness.dashboard("delivery").await;
    harness.file("delivery", &id).await;

    let removed = harness.unfile("delivery", &id).await;
    let again = harness.unfile("delivery", &id).await;

    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(again.status, StatusCode::NO_CONTENT);
    assert_eq!(harness.folders_of("delivery").await, json!([]));
}

#[tokio::test]
async fn a_dashboard_has_no_single_folder_to_set() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;

    let refused = harness
        .send(
            "PUT",
            "/v1/dashboards/delivery/folder",
            Some(json!({ "folder": null })),
        )
        .await;

    assert_eq!(refused.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn moving_into_a_folder_that_went_away_is_not_found_and_moves_nothing() {
    let harness = Harness::new();
    let platform = harness.created_id("Platform").await;
    let gone = harness.created_id("Gone").await;
    harness.dashboard("delivery").await;
    harness.file("delivery", &platform).await;
    harness
        .send("DELETE", &format!("/v1/folders/{gone}"), None)
        .await;

    let moved = harness.file("delivery", &gone).await;

    assert_eq!(moved.status, StatusCode::NOT_FOUND, "{}", moved.body);
    assert_eq!(
        harness.folders_of("delivery").await,
        json!([{ "id": platform, "name": "Platform" }])
    );
}

#[tokio::test]
async fn moving_a_dashboard_that_is_not_there_is_not_found() {
    let harness = Harness::new();
    let id = harness.created_id("Platform").await;

    let moved = harness.file("nowhere", &id).await;

    assert_eq!(moved.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn moving_into_a_folder_id_that_is_not_a_uuid_is_not_found() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;

    let moved = harness.file("delivery", "platform").await;

    assert_eq!(moved.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_dashboard_list_narrows_to_one_folder_or_to_the_unfiled() {
    let harness = Harness::new();
    let id = harness.created_id("Platform").await;
    for name in ["delivery", "delivery_costs", "hiring"] {
        harness.dashboard(name).await;
    }
    harness.file("delivery", &id).await;

    let filed = harness
        .send("GET", &format!("/v1/dashboards?folder={id}"), None)
        .await;
    let unfiled = harness
        .send("GET", "/v1/dashboards?folder=unfiled&q=%20deliv%20", None)
        .await;

    assert_eq!(filed.status, StatusCode::OK);
    assert_eq!(listed_ids(&filed), ["delivery"], "{}", filed.body);
    assert_eq!(filed.body["total"], 1);
    assert_eq!(listed_ids(&unfiled), ["delivery_costs"], "{}", unfiled.body);
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
    assert!(read.body.get("folder").is_none(), "{}", read.body);
}

#[tokio::test]
async fn a_caller_identity_refuses_is_refused_on_every_folder_route() {
    let harness = Harness::with_identity(
        crate::store::identity::IdentityClient::unidentified(),
        Arc::new(MemoryDefinitions::new()),
    );
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
        ("PUT", format!("/v1/folders/{id}/dashboards/delivery"), None),
        (
            "DELETE",
            format!("/v1/folders/{id}/dashboards/delivery"),
            None,
        ),
        ("GET", "/v1/dashboards?folder=unfiled".to_owned(), None),
    ];

    for (method, path, body) in routes {
        let refused = harness.send(method, &path, body).await;

        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{method} {path}");
    }
}

#[tokio::test]
async fn a_member_makes_and_lists_folders_of_their_own() {
    let harness = Harness::build(false, Arc::new(MemoryDefinitions::new()));

    let created = harness.create("Mine").await;
    let listed = harness.send("GET", "/v1/folders", None).await;

    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.body["access"], json!("owner"));
    assert_eq!(listed.body["folders"][0]["id"], created.body["id"]);
}

#[tokio::test]
async fn a_store_that_is_down_answers_a_server_error() {
    let harness = Harness::build(true, Arc::new(MemoryDefinitions::refusing()));

    let refused = harness.create("Platform").await;

    assert_eq!(refused.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn a_folder_past_the_cap_is_refused_and_says_why() {
    let harness = Harness::new();
    for index in 0..crate::domain::folders::MAX_FOLDERS {
        harness.created_id(&format!("f{index}")).await;
    }

    let refused = harness.create("one more").await;

    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(
        refused.body.to_string().contains("200 folders"),
        "{}",
        refused.body
    );
}

#[tokio::test]
async fn an_owned_dashboard_is_filed_in_my_folders_and_an_ownerless_one_in_ownerless_folders() {
    let harness = Harness::new();
    let mine = harness.created_id("Mine").await;
    harness.dashboard("delivery").await;
    harness.ownerless("legacy").await;

    let owned_in_mine = harness.file("delivery", &mine).await;
    let ownerless_in_mine = harness.file("legacy", &mine).await;

    assert_eq!(
        owned_in_mine.status,
        StatusCode::OK,
        "{}",
        owned_in_mine.body
    );
    assert_eq!(
        ownerless_in_mine.status,
        StatusCode::CONFLICT,
        "{}",
        ownerless_in_mine.body
    );
    assert!(
        ownerless_in_mine.body.to_string().contains("folder"),
        "{}",
        ownerless_in_mine.body
    );
    assert_eq!(harness.folders_of("legacy").await, json!([]));
}

#[tokio::test]
async fn someone_elses_folder_is_not_found_to_file_in_rename_or_remove() {
    let harness = Harness::new();
    let theirs = harness
        .send_as(
            SOMEONE_ELSE,
            "POST",
            "/v1/folders",
            Some(json!({ "name": "Theirs" })),
        )
        .await;
    let theirs = theirs.body["id"].as_str().unwrap_or_default().to_owned();
    harness.dashboard("delivery").await;

    let filed = harness.file("delivery", &theirs).await;
    let renamed = harness
        .send(
            "PATCH",
            &format!("/v1/folders/{theirs}"),
            Some(json!({ "name": "Mine" })),
        )
        .await;
    let removed = harness
        .send("DELETE", &format!("/v1/folders/{theirs}"), None)
        .await;
    let listed = harness.send("GET", "/v1/folders", None).await;

    for (what, answer) in [("file", &filed), ("rename", &renamed), ("remove", &removed)] {
        assert_eq!(
            answer.status,
            StatusCode::NOT_FOUND,
            "{what}: {}",
            answer.body
        );
    }
    assert_eq!(listed.body["folders"], json!([]));
}

#[tokio::test]
async fn each_administrator_files_a_shared_dashboard_in_their_own_folder() {
    let harness = Harness::new();
    harness.dashboard("delivery").await;
    let shared = harness
        .send(
            "PUT",
            &format!("/v1/dashboards/delivery/grants/{SOMEONE_ELSE}"),
            Some(json!({ "level": "viewer" })),
        )
        .await;
    let mine = harness.created_id("Mine").await;
    let theirs = harness
        .send_as(
            SOMEONE_ELSE,
            "POST",
            "/v1/folders",
            Some(json!({ "name": "Theirs" })),
        )
        .await;
    let theirs = theirs.body["id"].as_str().unwrap_or_default().to_owned();

    harness.file("delivery", &mine).await;
    let filed_by_viewer = harness
        .send_as(
            SOMEONE_ELSE,
            "PUT",
            &format!("/v1/folders/{theirs}/dashboards/delivery"),
            None,
        )
        .await;
    let seen_by_viewer = harness
        .send_as(SOMEONE_ELSE, "GET", "/v1/dashboards/delivery", None)
        .await;

    assert_eq!(shared.status, StatusCode::NO_CONTENT, "{}", shared.body);
    assert_eq!(filed_by_viewer.body, json!({ "carries": false }));
    assert_eq!(
        seen_by_viewer.body["folders"],
        json!([{ "id": theirs, "name": "Theirs" }])
    );
    assert_eq!(
        harness.folders_of("delivery").await,
        json!([{ "id": mine, "name": "Mine" }])
    );
}

#[tokio::test]
async fn the_folder_cap_is_per_administrator() {
    let harness = Harness::new();
    for index in 0..crate::domain::folders::MAX_FOLDERS {
        harness.created_id(&format!("f{index}")).await;
    }

    let mine = harness.create("one more").await;
    let theirs = harness
        .send_as(
            SOMEONE_ELSE,
            "POST",
            "/v1/folders",
            Some(json!({ "name": "one more" })),
        )
        .await;

    assert_eq!(mine.status, StatusCode::CONFLICT, "{}", mine.body);
    assert_eq!(theirs.status, StatusCode::CREATED, "{}", theirs.body);
}

#[tokio::test]
async fn a_folder_counts_only_the_dashboards_its_owner_sees() {
    let harness = Harness::new();
    let mine = harness.created_id("Mine").await;
    harness.dashboard("delivery").await;
    harness.file("delivery", &mine).await;
    let parsed = DefinitionId::parse("private").unwrap_or_else(|error| panic!("{error}"));
    let someone = PersonId::parse(SOMEONE_ELSE).unwrap_or_else(|| panic!("a person id"));
    harness
        .definitions
        .seed_owned(
            &parsed,
            &json!({ "title": "private", "widgets": [] }),
            someone,
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let listed = harness.send("GET", "/v1/folders", None).await;

    assert_eq!(
        listed.body["folders"][0]["dashboards"], 1,
        "{}",
        listed.body
    );
    assert_eq!(listed.body["unfiled"], 0, "{}", listed.body);
}
