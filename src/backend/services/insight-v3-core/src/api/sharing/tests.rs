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
use crate::domain::access::PersonId;
use crate::domain::definition::{DefinitionId, DefinitionKind};
use crate::domain::query::metric_query::{MetricRunner, People};
use crate::store::definitions::memory::MemoryDefinitions;
use crate::store::identity::IdentityClient;

mod visibility;

pub(super) const OWNER: &str = "0192a1b2-0000-7000-8000-00000000000a";
pub(super) const VIEWER: &str = "0192a1b2-0000-7000-8000-00000000000b";
/// An administrator who neither owns nor was shared anything.
pub(super) const STRANGER: &str = "0192a1b2-0000-7000-8000-00000000000c";
/// Someone who no longer holds the administrator role.
pub(super) const FORMER: &str = "0192a1b2-0000-7000-8000-00000000000d";

pub(super) fn person(raw: &str) -> PersonId {
    PersonId::parse(raw).unwrap_or_else(|| panic!("`{raw}` is a person id"))
}

fn administrators() -> Vec<Uuid> {
    [OWNER, VIEWER, STRANGER]
        .iter()
        .map(|raw| person(raw).uuid())
        .collect()
}

fn commits() -> Value {
    json!({
        "title": "Commits",
        "fields": [{ "name": "day", "path": "day", "type": "string" }]
    })
}

pub(super) struct Harness {
    _clickhouse: Mock,
    router: Router,
    pub(super) definitions: Arc<MemoryDefinitions>,
}

#[derive(Debug)]
pub(super) struct Answer {
    pub(super) status: StatusCode,
    pub(super) body: Value,
}

impl Harness {
    pub(super) fn new() -> Self {
        Self::build(IdentityClient::fixed(true).with_administrators(&administrators()))
    }

    fn with_roster_cut_short() -> Self {
        Self::build(IdentityClient::fixed(true).with_administrators_cut_short(&administrators()))
    }

    fn build(identity: IdentityClient) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let url = mock.url();
        let openapi = OpenApiRegistryImpl::new();
        let definitions = Arc::new(MemoryDefinitions::new());
        let state = Arc::new(AppState::new(
            MetricRunner::new(
                insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "insight")),
                People::new("identity"),
            ),
            definitions.clone(),
            ChatClient::keyless(),
            identity,
            crate::api::Datasets::holding(url, &[("commits", commits())]),
            crate::store::catalog::Catalog::fixed(Vec::new()),
        ));

        let router = crate::api::definitions::register_routes(Router::new(), &openapi, &state);
        let router = crate::api::duplicate::register_routes(router, &openapi, &state);
        let router = super::register_routes(router, &openapi, &state);
        let router = crate::api::folders::register_routes(router, &openapi, &state);
        let router = crate::api::tags::register_routes(router, &openapi, &state);
        let router = crate::api::pins::register_routes(router, &openapi, &state);

        Self {
            _clickhouse: mock,
            router,
            definitions,
        }
    }

    pub(super) async fn send(
        &self,
        caller: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Answer {
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

    /// A dashboard `owner` makes through the route, by its new id.
    pub(super) async fn dashboard_of(&self, owner: &str, name: &str) -> String {
        self.made(
            owner,
            "dashboards",
            name,
            json!({ "title": name, "widgets": [] }),
        )
        .await
    }

    pub(super) async fn made(&self, caller: &str, kind: &str, name: &str, body: Value) -> String {
        let answer = self
            .send(
                caller,
                "POST",
                &format!("/v1/{kind}"),
                Some(json!({ "name": name, "body": body })),
            )
            .await;
        assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.body);

        answer.body["id"]
            .as_str()
            .unwrap_or_else(|| panic!("a create answers its id: {}", answer.body))
            .to_owned()
    }

    /// A dashboard stored before ownership, under an id the case chooses.
    pub(super) async fn ownerless(&self, id: &str) {
        let parsed = DefinitionId::parse(id).unwrap_or_else(|error| panic!("{error}"));
        self.definitions
            .seed(
                DefinitionKind::Dashboard,
                &parsed,
                &json!({ "title": id, "widgets": [] }),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }

    /// A dashboard held by `owner` without going through a route, for an
    /// owner who can no longer reach one.
    async fn held_by(&self, owner: &str, id: &str) {
        let parsed = DefinitionId::parse(id).unwrap_or_else(|error| panic!("{error}"));
        self.definitions
            .seed_owned(
                &parsed,
                &json!({ "title": id, "widgets": [] }),
                person(owner),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }

    pub(super) async fn share(&self, caller: &str, dashboard: &str, with: &str) -> Answer {
        self.send(
            caller,
            "PUT",
            &format!("/v1/dashboards/{dashboard}/shares/{with}"),
            None,
        )
        .await
    }

    async fn shares(&self, caller: &str, dashboard: &str) -> Answer {
        self.send(
            caller,
            "GET",
            &format!("/v1/dashboards/{dashboard}/shares"),
            None,
        )
        .await
    }

    async fn withdraw(&self, caller: &str, dashboard: &str, from: &str) -> Answer {
        self.send(
            caller,
            "DELETE",
            &format!("/v1/dashboards/{dashboard}/shares/{from}"),
            None,
        )
        .await
    }

    async fn transfer(&self, caller: &str, dashboard: &str, to: &str) -> Answer {
        self.send(
            caller,
            "POST",
            &format!("/v1/dashboards/{dashboard}/transfer"),
            Some(json!({ "to": to })),
        )
        .await
    }

    async fn orphaned(&self, caller: &str) -> Answer {
        self.send(caller, "GET", "/v1/dashboards/orphaned", None)
            .await
    }

    pub(super) async fn read(&self, caller: &str, dashboard: &str) -> Answer {
        self.send(caller, "GET", &format!("/v1/dashboards/{dashboard}"), None)
            .await
    }
}

pub(super) fn field_of(answer: &Answer) -> Value {
    answer.body["context"]["field_violations"][0]["field"].clone()
}

#[tokio::test]
async fn the_owner_shares_with_an_administrator_once_however_often_asked() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;

    let first = harness.share(OWNER, &board, VIEWER).await;
    let again = harness.share(OWNER, &board, VIEWER).await;
    let listed = harness.shares(OWNER, &board).await;

    assert_eq!(first.status, StatusCode::NO_CONTENT, "{}", first.body);
    assert_eq!(again.status, StatusCode::NO_CONTENT, "{}", again.body);
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    let items = listed.body["items"].as_array().cloned().unwrap_or_default();
    assert_eq!(items.len(), 1, "{}", listed.body);
    assert_eq!(items[0]["person_id"], json!(VIEWER));
    assert_eq!(items[0]["granted_by"], json!(OWNER));
    assert!(items[0]["created_at"].is_string(), "{}", listed.body);
    assert_eq!(
        harness.read(VIEWER, &board).await.body["access"],
        json!("viewer")
    );
}

#[tokio::test]
async fn a_share_with_the_owner_or_someone_no_longer_an_administrator_is_refused_on_person_id() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;

    for target in [OWNER, FORMER, "someone"] {
        let refused = harness.share(OWNER, &board, target).await;

        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{target}: {}",
            refused.body
        );
        assert_eq!(field_of(&refused), json!("person_id"), "{target}");
    }
    assert_eq!(harness.shares(OWNER, &board).await.body["items"], json!([]));
}

#[tokio::test]
async fn only_the_owner_shares_lists_or_withdraws() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness.share(OWNER, &board, VIEWER).await;

    let cases = [
        (VIEWER, StatusCode::FORBIDDEN),
        (STRANGER, StatusCode::NOT_FOUND),
    ];
    for (caller, expected) in cases {
        let shared = harness.share(caller, &board, STRANGER).await;
        let listed = harness.shares(caller, &board).await;
        let withdrawn = harness.withdraw(caller, &board, VIEWER).await;

        for answer in [&shared, &listed, &withdrawn] {
            assert_eq!(answer.status, expected, "{caller}: {}", answer.body);
        }
    }
    let viewer = harness.share(VIEWER, &board, STRANGER).await;
    assert!(
        viewer
            .body
            .to_string()
            .contains("only the owner can change this dashboard"),
        "{}",
        viewer.body
    );
}

#[tokio::test]
async fn an_ownerless_dashboard_is_never_shared() {
    let harness = Harness::new();
    harness.ownerless("legacy").await;

    let refused = harness.share(OWNER, "legacy", VIEWER).await;
    let listed = harness.shares(OWNER, "legacy").await;

    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert_eq!(listed.status, StatusCode::CONFLICT, "{}", listed.body);
}

#[tokio::test]
async fn withdrawing_a_share_takes_the_viewers_pin_and_folder_with_it_and_repeats_harmlessly() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness.share(OWNER, &board, VIEWER).await;
    let pinned = harness
        .send(VIEWER, "PUT", &format!("/v1/pins/{board}"), None)
        .await;
    let folder = harness
        .send(
            VIEWER,
            "POST",
            "/v1/folders",
            Some(json!({ "name": "Mine" })),
        )
        .await;
    let filed = harness
        .send(
            VIEWER,
            "PUT",
            &format!("/v1/dashboards/{board}/folder"),
            Some(json!({ "folder": folder.body["id"] })),
        )
        .await;

    let withdrawn = harness.withdraw(OWNER, &board, VIEWER).await;
    let again = harness.withdraw(OWNER, &board, VIEWER).await;
    harness.share(OWNER, &board, VIEWER).await;

    assert_eq!(pinned.status, StatusCode::NO_CONTENT, "{}", pinned.body);
    assert_eq!(filed.status, StatusCode::NO_CONTENT, "{}", filed.body);
    assert_eq!(
        withdrawn.status,
        StatusCode::NO_CONTENT,
        "{}",
        withdrawn.body
    );
    assert_eq!(again.status, StatusCode::NO_CONTENT, "{}", again.body);
    let pins = harness.send(VIEWER, "GET", "/v1/pins", None).await;
    assert_eq!(pins.body["pins"], json!([]));
    assert_eq!(
        harness.read(VIEWER, &board).await.body["folder"],
        Value::Null
    );
}

#[tokio::test]
async fn a_withdrawn_viewer_finds_the_dashboard_missing() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness.share(OWNER, &board, VIEWER).await;

    harness.withdraw(OWNER, &board, VIEWER).await;

    assert_eq!(
        harness.read(VIEWER, &board).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_dashboard_whose_owner_left_is_orphaned_and_handed_to_an_administrator() {
    let harness = Harness::new();
    harness.held_by(FORMER, "left_behind").await;
    harness.dashboard_of(OWNER, "Kept").await;

    let listed = harness.orphaned(STRANGER).await;
    let handed = harness.transfer(STRANGER, "left_behind", VIEWER).await;
    let after = harness.orphaned(STRANGER).await;

    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert_eq!(listed.body["total"], 1);
    assert_eq!(listed.body["items"][0]["id"], json!("left_behind"));
    assert_eq!(listed.body["items"][0]["owner_person_id"], json!(FORMER));
    assert!(
        listed.body["items"][0]["updated_at"].is_string(),
        "{}",
        listed.body
    );
    assert_eq!(handed.status, StatusCode::NO_CONTENT, "{}", handed.body);
    let read = harness.read(VIEWER, "left_behind").await;
    assert_eq!(read.body["access"], json!("owner"));
    assert_eq!(read.body["owner_person_id"], json!(VIEWER));
    assert_eq!(after.body["items"], json!([]));
}

#[tokio::test]
async fn a_transfer_drops_the_share_the_new_owner_held() {
    let harness = Harness::new();
    harness.held_by(FORMER, "left_behind").await;
    let parsed = DefinitionId::parse("left_behind").unwrap_or_else(|error| panic!("{error}"));
    crate::domain::sharing::Sharing::share(
        harness.definitions.as_ref(),
        &parsed,
        person(VIEWER),
        person(FORMER),
    )
    .await
    .unwrap_or_else(|error| panic!("{error}"));

    let handed = harness.transfer(STRANGER, "left_behind", VIEWER).await;

    assert_eq!(handed.status, StatusCode::NO_CONTENT, "{}", handed.body);
    assert_eq!(
        harness.shares(VIEWER, "left_behind").await.body["items"],
        json!([])
    );
}

#[tokio::test]
async fn a_dashboard_whose_owner_is_an_administrator_is_refused_where_seen_and_missing_where_not() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness.share(OWNER, &board, VIEWER).await;

    let seen = harness.transfer(VIEWER, &board, STRANGER).await;
    let unseen = harness.transfer(STRANGER, &board, VIEWER).await;
    let missing = harness.transfer(STRANGER, "nowhere", VIEWER).await;

    assert_eq!(seen.status, StatusCode::CONFLICT, "{}", seen.body);
    assert_eq!(unseen.status, StatusCode::NOT_FOUND, "{}", unseen.body);
    assert_eq!(
        unseen.body.to_string().replace(&board, "nowhere"),
        missing.body.to_string(),
        "a private dashboard answers as a missing one"
    );
}

#[tokio::test]
async fn an_ownerless_dashboard_is_never_transferred() {
    let harness = Harness::new();
    harness.ownerless("legacy").await;

    let refused = harness.transfer(STRANGER, "legacy", VIEWER).await;

    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
}

#[tokio::test]
async fn a_transfer_to_someone_who_is_not_an_administrator_is_refused_on_to() {
    let harness = Harness::new();
    harness.held_by(FORMER, "left_behind").await;

    for to in [FORMER, "someone"] {
        let refused = harness.transfer(STRANGER, "left_behind", to).await;

        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{to}: {}",
            refused.body
        );
        assert_eq!(field_of(&refused), json!("to"), "{to}");
    }
}

#[tokio::test]
async fn an_administrator_listing_that_may_be_cut_short_is_refused_not_read() {
    let harness = Harness::with_roster_cut_short();
    harness.held_by(FORMER, "left_behind").await;

    let refused = harness.orphaned(STRANGER).await;

    assert_eq!(
        refused.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        refused.body
    );
}

#[tokio::test]
async fn the_owner_withdrawing_from_themselves_is_refused_and_keeps_their_pin() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness
        .send(OWNER, "PUT", &format!("/v1/pins/{board}"), None)
        .await;

    let refused = harness.withdraw(OWNER, &board, OWNER).await;

    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    assert_eq!(field_of(&refused), json!("person_id"));
    let pins = harness.send(OWNER, "GET", "/v1/pins", None).await;
    assert_eq!(pins.body["pins"][0]["id"], json!(board));
}

#[tokio::test]
async fn an_administrator_listing_that_leaves_out_the_caller_is_refused_not_read() {
    let harness = Harness::build(IdentityClient::fixed(true).with_administrators(&[]));
    harness.held_by(FORMER, "left_behind").await;

    let refused = harness.orphaned(STRANGER).await;

    assert_eq!(
        refused.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        refused.body
    );
}
