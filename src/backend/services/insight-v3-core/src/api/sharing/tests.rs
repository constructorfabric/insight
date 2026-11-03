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

mod folders;
mod members;
mod visibility;

pub(super) const OWNER: &str = "0192a1b2-0000-7000-8000-00000000000a";
pub(super) const VIEWER: &str = "0192a1b2-0000-7000-8000-00000000000b";
/// An administrator who neither owns nor was shared anything.
pub(super) const STRANGER: &str = "0192a1b2-0000-7000-8000-00000000000c";
/// Someone who no longer holds the administrator role.
pub(super) const FORMER: &str = "0192a1b2-0000-7000-8000-00000000000d";
/// A signed-in person without the administrator role.
pub(super) const MEMBER: &str = "0192a1b2-0000-7000-8000-00000000000e";
/// A person identity does not know.
pub(super) const UNKNOWN: &str = "0192a1b2-0000-7000-8000-00000000000f";

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
    clickhouse: Mock,
    router: Router,
    pub(super) definitions: Arc<MemoryDefinitions>,
    pub(super) state: Arc<AppState>,
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
        let identity = identity
            .with_members(&[person(MEMBER).uuid(), person(FORMER).uuid()])
            .with_strangers(&[person(UNKNOWN).uuid()]);
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
        let router = crate::api::dashboard_widgets::register_routes(router, &openapi, &state);

        Self {
            clickhouse: mock,
            router,
            definitions,
            state,
        }
    }

    /// The next warehouse read fails, as a warehouse that is down does.
    pub(super) fn failing_read(&self) {
        self.clickhouse.add(clickhouse::test::handlers::failure(
            clickhouse::test::status::INTERNAL_SERVER_ERROR,
        ));
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
    pub(super) async fn held_by(&self, owner: &str, id: &str) {
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

    /// Grants a dashboard to `with` as a viewer.
    pub(super) async fn share(&self, caller: &str, dashboard: &str, with: &str) -> Answer {
        self.grant(caller, "dashboards", dashboard, with, "viewer")
            .await
    }

    /// Grants a dashboard or a folder, by its segment, at `level`.
    pub(super) async fn grant(
        &self,
        caller: &str,
        segment: &str,
        id: &str,
        grantee: &str,
        level: &str,
    ) -> Answer {
        self.send(
            caller,
            "PUT",
            &format!("/v1/{segment}/{id}/grants/{grantee}"),
            Some(json!({ "level": level })),
        )
        .await
    }

    pub(super) async fn shares(&self, caller: &str, dashboard: &str) -> Answer {
        self.send(
            caller,
            "GET",
            &format!("/v1/dashboards/{dashboard}/grants"),
            None,
        )
        .await
    }

    pub(super) async fn withdraw(&self, caller: &str, dashboard: &str, from: &str) -> Answer {
        self.send(
            caller,
            "DELETE",
            &format!("/v1/dashboards/{dashboard}/grants/{from}"),
            None,
        )
        .await
    }

    /// A folder `owner` makes through the route, by its new id.
    pub(super) async fn folder_of(&self, owner: &str, name: &str) -> String {
        let made = self
            .send(owner, "POST", "/v1/folders", Some(json!({ "name": name })))
            .await;
        assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);

        made.body["id"]
            .as_str()
            .unwrap_or_else(|| panic!("a folder answers its id: {}", made.body))
            .to_owned()
    }

    /// Puts the dashboard in the folder as `caller`.
    pub(super) async fn add(&self, caller: &str, folder: &str, dashboard: &str) -> Answer {
        self.send(
            caller,
            "PUT",
            &format!("/v1/folders/{folder}/dashboards/{dashboard}"),
            None,
        )
        .await
    }

    pub(super) async fn transfer(&self, caller: &str, dashboard: &str, to: &str) -> Answer {
        self.send(
            caller,
            "POST",
            &format!("/v1/dashboards/{dashboard}/transfer"),
            Some(json!({ "to": to })),
        )
        .await
    }

    async fn orphaned(&self, caller: &str) -> Answer {
        self.send(caller, "GET", "/v1/orphaned-dashboards", None)
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
async fn granting_again_sets_the_level_and_keeps_one_grant() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;

    let first = harness.share(OWNER, &board, VIEWER).await;
    let again = harness.share(OWNER, &board, VIEWER).await;
    let raised = harness
        .grant(OWNER, "dashboards", &board, VIEWER, "editor")
        .await;
    let listed = harness.shares(OWNER, &board).await;

    for answer in [&first, &again, &raised] {
        assert_eq!(answer.status, StatusCode::NO_CONTENT, "{}", answer.body);
    }
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    let items = listed.body["items"].as_array().cloned().unwrap_or_default();
    assert_eq!(items.len(), 1, "{}", listed.body);
    assert_eq!(items[0]["grantee"], json!(VIEWER));
    assert_eq!(items[0]["level"], json!("editor"));
    assert_eq!(items[0]["granted_by"], json!(OWNER));
    assert!(items[0]["created_at"].is_string(), "{}", listed.body);
    assert_eq!(
        harness.read(VIEWER, &board).await.body["access"],
        json!("editor")
    );
}

#[tokio::test]
async fn a_listing_shows_how_far_each_dashboards_own_grants_reach() {
    let harness = Harness::new();
    let private = harness.dashboard_of(OWNER, "Private").await;
    let people = harness.dashboard_of(OWNER, "People").await;
    let everyone = harness.dashboard_of(OWNER, "Everyone").await;
    harness.share(OWNER, &people, VIEWER).await;
    harness.share(OWNER, &everyone, VIEWER).await;
    harness.share(OWNER, &everyone, "everyone").await;

    let listed = harness.send(OWNER, "GET", "/v1/dashboards", None).await;

    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    let items = listed.body["items"].as_array().cloned().unwrap_or_default();
    for (id, visibility) in [
        (&private, "private"),
        (&people, "people"),
        (&everyone, "everyone"),
    ] {
        let item = items
            .iter()
            .find(|item| item["id"] == json!(id))
            .unwrap_or_else(|| panic!("{id} is listed: {}", listed.body));
        assert_eq!(item["visibility"], json!(visibility), "{id}: {item}");
        assert!(item["updated_at"].is_string(), "{id}: {item}");
    }
}

#[tokio::test]
async fn a_grant_to_the_owner_an_unknown_person_or_no_person_is_refused_on_grantee() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;

    for target in [OWNER, UNKNOWN, "someone"] {
        let refused = harness.share(OWNER, &board, target).await;

        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{target}: {}",
            refused.body
        );
        assert_eq!(field_of(&refused), json!("grantee"), "{target}");
    }
    let bad_level = harness
        .grant(OWNER, "dashboards", &board, VIEWER, "owner")
        .await;
    assert_eq!(field_of(&bad_level), json!("level"));
    assert_eq!(harness.shares(OWNER, &board).await.body["items"], json!([]));
}

#[tokio::test]
async fn any_person_identity_knows_is_a_share_target_administrator_or_not() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;

    let to_member = harness.share(OWNER, &board, MEMBER).await;
    let to_former = harness.share(OWNER, &board, FORMER).await;

    assert_eq!(
        to_member.status,
        StatusCode::NO_CONTENT,
        "{}",
        to_member.body
    );
    assert_eq!(
        to_former.status,
        StatusCode::NO_CONTENT,
        "{}",
        to_former.body
    );
}

#[tokio::test]
async fn the_owner_and_an_editor_share_onward_and_a_viewer_or_stranger_may_not() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness.share(OWNER, &board, VIEWER).await;
    harness
        .grant(OWNER, "dashboards", &board, STRANGER, "editor")
        .await;

    let onward = harness.share(STRANGER, &board, MEMBER).await;
    let listed = harness.shares(STRANGER, &board).await;
    let withdrawn_by_editor = harness.withdraw(STRANGER, &board, MEMBER).await;
    let by_viewer = harness.share(VIEWER, &board, MEMBER).await;
    let listed_by_viewer = harness.shares(VIEWER, &board).await;
    let by_member = harness.share(MEMBER, &board, VIEWER).await;

    assert_eq!(onward.status, StatusCode::NO_CONTENT, "{}", onward.body);
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert_eq!(
        withdrawn_by_editor.status,
        StatusCode::NO_CONTENT,
        "{}",
        withdrawn_by_editor.body
    );
    for refused in [&by_viewer, &listed_by_viewer] {
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
        assert!(
            refused
                .body
                .to_string()
                .contains("only the owner or an editor can change this dashboard"),
            "{}",
            refused.body
        );
    }
    assert_eq!(
        by_member.status,
        StatusCode::FORBIDDEN,
        "{}",
        by_member.body
    );
    assert!(
        by_member.body.to_string().contains("admin role required"),
        "{}",
        by_member.body
    );
}

#[tokio::test]
async fn a_dashboard_shared_with_everyone_is_seen_by_every_signed_in_person() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;

    let before = harness.read(MEMBER, &board).await;
    let shared = harness
        .grant(OWNER, "dashboards", &board, "everyone", "editor")
        .await;
    let by_member = harness.read(MEMBER, &board).await;
    let by_admin = harness.read(STRANGER, &board).await;
    let withdrawn = harness.withdraw(OWNER, &board, "everyone").await;
    let after = harness.read(STRANGER, &board).await;

    assert_eq!(before.status, StatusCode::NOT_FOUND, "{}", before.body);
    assert_eq!(shared.status, StatusCode::NO_CONTENT, "{}", shared.body);
    assert_eq!(by_member.body["access"], json!("viewer"));
    assert_eq!(by_admin.body["access"], json!("editor"));
    assert_eq!(
        withdrawn.status,
        StatusCode::NO_CONTENT,
        "{}",
        withdrawn.body
    );
    assert_eq!(after.status, StatusCode::NOT_FOUND, "{}", after.body);
    let listed = harness.shares(OWNER, &board).await;
    assert_eq!(listed.body["items"], json!([]));
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
async fn withdrawing_a_grant_hides_the_pin_and_entry_and_granting_again_shows_them() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness.share(OWNER, &board, MEMBER).await;
    let pinned = harness
        .send(MEMBER, "PUT", &format!("/v1/pins/{board}"), None)
        .await;
    let folder = harness.folder_of(MEMBER, "Mine").await;
    let filed = harness.add(MEMBER, &folder, &board).await;

    let withdrawn = harness.withdraw(OWNER, &board, MEMBER).await;
    let again = harness.withdraw(OWNER, &board, MEMBER).await;
    let hidden_pins = harness.send(MEMBER, "GET", "/v1/pins", None).await;
    let hidden_folders = harness.send(MEMBER, "GET", "/v1/folders", None).await;
    harness.share(OWNER, &board, MEMBER).await;

    assert_eq!(pinned.status, StatusCode::NO_CONTENT, "{}", pinned.body);
    assert_eq!(filed.status, StatusCode::OK, "{}", filed.body);
    assert_eq!(filed.body, json!({ "carries": false }));
    assert_eq!(
        withdrawn.status,
        StatusCode::NO_CONTENT,
        "{}",
        withdrawn.body
    );
    assert_eq!(again.status, StatusCode::NO_CONTENT, "{}", again.body);
    assert_eq!(hidden_pins.body["pins"], json!([]));
    assert_eq!(hidden_folders.body["folders"][0]["dashboards"], 0);
    let pins = harness.send(MEMBER, "GET", "/v1/pins", None).await;
    assert_eq!(pins.body["pins"][0]["id"], json!(board));
    assert_eq!(
        harness.read(MEMBER, &board).await.body["folders"],
        json!([{ "id": folder, "name": "Mine" }])
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
async fn a_transfer_drops_the_grant_the_new_owner_held() {
    let harness = Harness::new();
    harness.held_by(FORMER, "left_behind").await;
    let parsed = DefinitionId::parse("left_behind").unwrap_or_else(|error| panic!("{error}"));
    harness
        .definitions
        .seed_viewer(&parsed, person(VIEWER), person(FORMER))
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
async fn withdrawing_a_grant_the_owner_never_held_changes_nothing_and_keeps_their_pin() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness
        .send(OWNER, "PUT", &format!("/v1/pins/{board}"), None)
        .await;

    let withdrawn = harness.withdraw(OWNER, &board, OWNER).await;

    assert_eq!(
        withdrawn.status,
        StatusCode::NO_CONTENT,
        "{}",
        withdrawn.body
    );
    assert_eq!(
        harness.read(OWNER, &board).await.body["access"],
        json!("owner")
    );
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

#[tokio::test]
async fn an_unreachable_identity_refuses_a_share_a_seen_transfer_and_the_orphaned_listing() {
    let harness = Harness::build(IdentityClient::fixed(true).with_administrators_unreachable());
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness.held_by(FORMER, "left_behind").await;
    let parsed = DefinitionId::parse("left_behind").unwrap_or_else(|error| panic!("{error}"));
    harness
        .definitions
        .seed_viewer(&parsed, person(STRANGER), person(FORMER))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let shared = harness.share(OWNER, &board, VIEWER).await;
    let handed = harness.transfer(STRANGER, "left_behind", VIEWER).await;
    let orphaned = harness.orphaned(STRANGER).await;

    for answer in [&shared, &handed, &orphaned] {
        assert_eq!(
            answer.status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{}",
            answer.body
        );
    }
    assert_eq!(harness.shares(OWNER, &board).await.body["items"], json!([]));
    assert_eq!(
        harness.read(STRANGER, "left_behind").await.body["owner_person_id"],
        json!(FORMER)
    );
}

#[tokio::test]
async fn an_unreachable_identity_answers_a_transfer_of_an_unseen_dashboard_as_missing() {
    let harness = Harness::build(IdentityClient::fixed(true).with_administrators_unreachable());
    harness.held_by(FORMER, "left_behind").await;

    let unseen = harness.transfer(STRANGER, "left_behind", VIEWER).await;
    let missing = harness.transfer(STRANGER, "nowhere", VIEWER).await;

    assert_eq!(unseen.status, StatusCode::NOT_FOUND, "{}", unseen.body);
    assert_eq!(
        unseen.body.to_string().replace("left_behind", "nowhere"),
        missing.body.to_string(),
        "a private dashboard answers as a missing one"
    );
}

#[tokio::test]
async fn a_transfer_whose_tags_would_overfill_the_new_owners_scope_is_refused_and_moves_nothing() {
    use crate::domain::access::Owner;
    use crate::domain::tags::{MAX_TAGS, MAX_TAGS_PER_DASHBOARD, TagSet, Tags as _};

    let harness = Harness::new();
    let store = harness.definitions.as_ref();
    for board in 0..MAX_TAGS / MAX_TAGS_PER_DASHBOARD {
        let id = harness.dashboard_of(VIEWER, &format!("Full {board}")).await;
        let names: Vec<String> = (0..MAX_TAGS_PER_DASHBOARD)
            .map(|tag| format!("tag {board} {tag}"))
            .collect();
        let tags = TagSet::parse(&names).unwrap_or_else(|error| panic!("{error}"));
        let parsed = DefinitionId::parse(&id).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_tags(&parsed, Owner::Person(person(VIEWER)), &tags)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
    harness.held_by(FORMER, "left_behind").await;
    let parsed = DefinitionId::parse("left_behind").unwrap_or_else(|error| panic!("{error}"));
    let extra = TagSet::parse(&["one more".to_owned()]).unwrap_or_else(|error| panic!("{error}"));
    store
        .set_tags(&parsed, Owner::Person(person(FORMER)), &extra)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let refused = harness.transfer(STRANGER, "left_behind", VIEWER).await;

    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(
        refused.body.to_string().contains("too many tags"),
        "{}",
        refused.body
    );
    assert_eq!(harness.orphaned(STRANGER).await.body["total"], 1);
    let kept = store
        .tags_of(&parsed)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(kept.len(), 1, "{kept:?}");
}
