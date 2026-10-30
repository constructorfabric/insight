//! Folders as shareable containers: carrying and filing entries, folder
//! grants, and orphaned folders.

use axum::http::StatusCode;
use serde_json::json;

use super::{FORMER, Harness, MEMBER, OWNER, STRANGER, VIEWER, person};
use crate::domain::folders::Folders as _;

fn names(listed: &super::Answer) -> Vec<String> {
    listed.body["items"]
        .as_array()
        .unwrap_or_else(|| panic!("a listing answers items: {}", listed.body))
        .iter()
        .map(|item| item["name"].as_str().unwrap_or_default().to_owned())
        .collect()
}

async fn in_folder(harness: &Harness, caller: &str, folder: &str) -> super::Answer {
    harness
        .send(
            caller,
            "GET",
            &format!("/v1/dashboards?folder={folder}"),
            None,
        )
        .await
}

#[tokio::test]
async fn a_carrying_entry_shows_the_dashboard_to_the_folder_s_viewers_and_editors() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    let team = harness.folder_of(OWNER, "Team").await;
    harness
        .grant(OWNER, "folders", &team, VIEWER, "editor")
        .await;
    harness
        .grant(OWNER, "folders", &team, MEMBER, "viewer")
        .await;

    let added = harness.add(OWNER, &team, &board).await;
    let by_editor = harness.read(VIEWER, &board).await;
    let by_member = harness.read(MEMBER, &board).await;
    let edited = harness
        .send(
            VIEWER,
            "PUT",
            &format!("/v1/dashboards/{board}"),
            Some(json!({ "name": "Edited", "body": { "title": "Edited", "widgets": [] } })),
        )
        .await;
    let listed = harness.send(MEMBER, "GET", "/v1/folders", None).await;
    let contents = in_folder(&harness, MEMBER, &team).await;

    assert_eq!(added.status, StatusCode::OK, "{}", added.body);
    assert_eq!(added.body, json!({ "carries": true }));
    assert_eq!(by_editor.body["access"], json!("editor"));
    assert_eq!(
        by_editor.body["carries"],
        json!(false),
        "an editor through a folder files rather than carries"
    );
    assert_eq!(by_member.body["access"], json!("viewer"));
    assert_eq!(edited.status, StatusCode::NO_CONTENT, "{}", edited.body);
    assert_eq!(
        listed.body["folders"],
        json!([{
            "id": team,
            "name": "Team",
            "owner_person_id": OWNER,
            "access": "viewer",
            "dashboards": 1
        }])
    );
    assert_eq!(names(&contents), ["Edited"]);
}

#[tokio::test]
async fn a_filing_entry_shows_the_dashboard_only_to_who_sees_it_anyway() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness.share(OWNER, &board, VIEWER).await;
    let theirs = harness.folder_of(VIEWER, "Theirs").await;
    harness
        .grant(VIEWER, "folders", &theirs, STRANGER, "viewer")
        .await;

    let filed = harness.add(VIEWER, &theirs, &board).await;
    let by_stranger = harness.read(STRANGER, &board).await;
    let stranger_contents = in_folder(&harness, STRANGER, &theirs).await;
    let viewer_contents = in_folder(&harness, VIEWER, &theirs).await;

    assert_eq!(filed.body, json!({ "carries": false }));
    assert_eq!(
        by_stranger.status,
        StatusCode::NOT_FOUND,
        "{}",
        by_stranger.body
    );
    assert_eq!(stranger_contents.body["total"], 0);
    assert_eq!(names(&viewer_contents), ["Board"]);
}

#[tokio::test]
async fn a_folder_editor_who_is_not_the_dashboard_s_editor_files_it_elsewhere() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    let team = harness.folder_of(OWNER, "Team").await;
    harness
        .grant(OWNER, "folders", &team, VIEWER, "editor")
        .await;
    harness.add(OWNER, &team, &board).await;
    let theirs = harness.folder_of(VIEWER, "Theirs").await;
    harness
        .grant(VIEWER, "folders", &theirs, STRANGER, "viewer")
        .await;

    let filed = harness.add(VIEWER, &theirs, &board).await;

    assert_eq!(filed.body, json!({ "carries": false }));
    assert_eq!(
        harness.read(STRANGER, &board).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn an_editor_s_entry_stops_carrying_once_their_grant_is_withdrawn() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness
        .grant(OWNER, "dashboards", &board, VIEWER, "editor")
        .await;
    let theirs = harness.folder_of(VIEWER, "Theirs").await;
    harness
        .grant(VIEWER, "folders", &theirs, STRANGER, "viewer")
        .await;
    let carried = harness.add(VIEWER, &theirs, &board).await;
    let seen_before = harness.read(STRANGER, &board).await;

    harness.withdraw(OWNER, &board, VIEWER).await;

    assert_eq!(carried.body, json!({ "carries": true }));
    assert_eq!(seen_before.body["access"], json!("viewer"));
    assert_eq!(
        harness.read(VIEWER, &board).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        harness.read(STRANGER, &board).await.status,
        StatusCode::NOT_FOUND
    );
    harness.share(OWNER, &board, VIEWER).await;
    assert_eq!(
        harness.read(STRANGER, &board).await.status,
        StatusCode::NOT_FOUND,
        "the entry was demoted to a filing and does not carry again"
    );
}

#[tokio::test]
async fn a_dashboard_sits_in_several_folders_and_leaves_one_without_leaving_the_others() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    let first = harness.folder_of(OWNER, "First").await;
    let second = harness.folder_of(OWNER, "Second").await;
    harness.add(OWNER, &first, &board).await;
    harness.add(OWNER, &second, &board).await;

    let both = harness.read(OWNER, &board).await;
    let removed = harness
        .send(
            OWNER,
            "DELETE",
            &format!("/v1/folders/{first}/dashboards/{board}"),
            None,
        )
        .await;
    let folder_gone = harness
        .send(OWNER, "DELETE", &format!("/v1/folders/{second}"), None)
        .await;
    let unfiled = harness
        .send(OWNER, "GET", "/v1/dashboards?folder=unfiled", None)
        .await;

    assert_eq!(
        both.body["folders"],
        json!([{ "id": first, "name": "First" }, { "id": second, "name": "Second" }])
    );
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    assert_eq!(
        folder_gone.status,
        StatusCode::NO_CONTENT,
        "{}",
        folder_gone.body
    );
    assert_eq!(names(&unfiled), ["Board"]);
    assert_eq!(harness.read(OWNER, &board).await.body["folders"], json!([]));
}

#[tokio::test]
async fn a_member_keeps_personal_folders_and_never_shares_them() {
    let harness = Harness::new();
    let mine = harness.folder_of(MEMBER, "Mine").await;

    let renamed = harness
        .send(
            MEMBER,
            "PATCH",
            &format!("/v1/folders/{mine}"),
            Some(json!({ "name": "Renamed" })),
        )
        .await;
    let shared = harness
        .grant(MEMBER, "folders", &mine, VIEWER, "viewer")
        .await;
    let deleted = harness
        .send(MEMBER, "DELETE", &format!("/v1/folders/{mine}"), None)
        .await;

    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    assert_eq!(renamed.body["access"], json!("owner"));
    assert_eq!(shared.status, StatusCode::FORBIDDEN, "{}", shared.body);
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
}

#[tokio::test]
async fn a_folder_viewer_neither_arranges_renames_nor_deletes_and_an_editor_does_not_delete() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    let team = harness.folder_of(OWNER, "Team").await;
    harness
        .grant(OWNER, "folders", &team, VIEWER, "viewer")
        .await;
    harness
        .grant(OWNER, "folders", &team, STRANGER, "editor")
        .await;

    let viewer_adds = harness.add(VIEWER, &team, &board).await;
    let viewer_renames = harness
        .send(
            VIEWER,
            "PATCH",
            &format!("/v1/folders/{team}"),
            Some(json!({ "name": "Mine" })),
        )
        .await;
    let editor_renames = harness
        .send(
            STRANGER,
            "PATCH",
            &format!("/v1/folders/{team}"),
            Some(json!({ "name": "Renamed" })),
        )
        .await;
    let editor_deletes = harness
        .send(STRANGER, "DELETE", &format!("/v1/folders/{team}"), None)
        .await;
    let editor_shares = harness
        .grant(STRANGER, "folders", &team, MEMBER, "viewer")
        .await;

    for refused in [&viewer_adds, &viewer_renames] {
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
        assert!(
            refused
                .body
                .to_string()
                .contains("only the owner or an editor can change this folder"),
            "{}",
            refused.body
        );
    }
    assert_eq!(
        editor_renames.status,
        StatusCode::OK,
        "{}",
        editor_renames.body
    );
    assert_eq!(
        editor_deletes.status,
        StatusCode::FORBIDDEN,
        "{}",
        editor_deletes.body
    );
    assert!(
        editor_deletes
            .body
            .to_string()
            .contains("only the owner can delete this folder"),
        "{}",
        editor_deletes.body
    );
    assert_eq!(
        editor_shares.status,
        StatusCode::NO_CONTENT,
        "{}",
        editor_shares.body
    );
}

#[tokio::test]
async fn an_orphaned_folder_is_listed_and_handed_to_an_administrator_with_its_grants() {
    let harness = Harness::new();
    let left = harness
        .definitions
        .create_folder(
            person(FORMER),
            crate::domain::folders::FolderName::parse("Left")
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let unshared = harness.folder_of(FORMER, "Unshared").await;
    let left_id = left.id.to_string();
    crate::domain::sharing::Grants::grant(
        harness.definitions.as_ref(),
        crate::domain::sharing::Shared::Folder(left.id),
        crate::domain::access::Grantee::Person(person(MEMBER)),
        crate::domain::access::Level::Viewer,
        person(FORMER),
    )
    .await
    .unwrap_or_else(|error| panic!("{error}"));

    let listed = harness
        .send(STRANGER, "GET", "/v1/orphaned-folders", None)
        .await;
    let not_orphaned = harness
        .send(
            STRANGER,
            "POST",
            &format!("/v1/folders/{unshared}/transfer"),
            Some(json!({ "to": VIEWER })),
        )
        .await;
    let to_member = harness
        .send(
            STRANGER,
            "POST",
            &format!("/v1/folders/{left_id}/transfer"),
            Some(json!({ "to": MEMBER })),
        )
        .await;
    let handed = harness
        .send(
            STRANGER,
            "POST",
            &format!("/v1/folders/{left_id}/transfer"),
            Some(json!({ "to": VIEWER })),
        )
        .await;
    let after = harness
        .send(STRANGER, "GET", "/v1/orphaned-folders", None)
        .await;
    let grants = harness
        .send(
            VIEWER,
            "GET",
            &format!("/v1/folders/{left_id}/grants"),
            None,
        )
        .await;

    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert_eq!(listed.body["total"], 1);
    assert_eq!(listed.body["items"][0]["id"], json!(left_id));
    assert_eq!(listed.body["items"][0]["owner_person_id"], json!(FORMER));
    assert_eq!(
        not_orphaned.status,
        StatusCode::NOT_FOUND,
        "a folder shared with no one is not orphaned, and the caller cannot see it: {}",
        not_orphaned.body
    );
    assert_eq!(
        to_member.status,
        StatusCode::BAD_REQUEST,
        "{}",
        to_member.body
    );
    assert_eq!(handed.status, StatusCode::NO_CONTENT, "{}", handed.body);
    assert_eq!(after.body["total"], 0);
    assert_eq!(grants.status, StatusCode::OK, "{}", grants.body);
    assert_eq!(grants.body["items"][0]["grantee"], json!(MEMBER));
}

#[tokio::test]
async fn an_ownerless_folder_is_the_administrators_and_holds_only_ownerless_dashboards() {
    let harness = Harness::new();
    harness.ownerless("legacy").await;
    let owned = harness.dashboard_of(OWNER, "Owned").await;
    let legacy = harness
        .definitions
        .seed_ownerless_folder("Legacy")
        .to_string();

    let fits = harness.add(STRANGER, &legacy, "legacy").await;
    let refused = harness.add(OWNER, &legacy, &owned).await;
    let shared = harness
        .grant(OWNER, "folders", &legacy, VIEWER, "viewer")
        .await;
    let by_member = harness.send(MEMBER, "GET", "/v1/folders", None).await;

    assert_eq!(fits.body, json!({ "carries": true }));
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert_eq!(shared.status, StatusCode::CONFLICT, "{}", shared.body);
    assert_eq!(by_member.body["folders"], json!([]));
}

#[tokio::test]
async fn an_editor_through_a_folder_edits_but_never_shares_the_dashboard() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    let team = harness.folder_of(OWNER, "Team").await;
    harness
        .grant(OWNER, "folders", &team, VIEWER, "editor")
        .await;
    harness.add(OWNER, &team, &board).await;
    harness
        .grant(OWNER, "dashboards", &board, STRANGER, "editor")
        .await;

    let edited = harness
        .send(
            VIEWER,
            "PUT",
            &format!("/v1/dashboards/{board}"),
            Some(json!({ "name": "Edited", "body": { "title": "Edited", "widgets": [] } })),
        )
        .await;
    let refused = [
        harness.shares(VIEWER, &board).await,
        harness.share(VIEWER, &board, MEMBER).await,
        harness.withdraw(VIEWER, &board, STRANGER).await,
    ];
    let direct_lists = harness.shares(STRANGER, &board).await;
    let direct_shares = harness.share(STRANGER, &board, MEMBER).await;

    assert_eq!(edited.status, StatusCode::NO_CONTENT, "{}", edited.body);
    for answer in &refused {
        assert_eq!(answer.status, StatusCode::FORBIDDEN, "{}", answer.body);
        assert!(
            answer.body.to_string().contains(
                "only the owner or an editor granted this dashboard directly or through everyone can share it"
            ),
            "{}",
            answer.body
        );
    }
    assert_eq!(direct_lists.status, StatusCode::OK, "{}", direct_lists.body);
    assert_eq!(
        direct_shares.status,
        StatusCode::NO_CONTENT,
        "{}",
        direct_shares.body
    );
}

#[tokio::test]
async fn adding_an_entry_answers_what_the_stored_entry_carries() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    let team = harness.folder_of(OWNER, "Team").await;
    harness
        .grant(OWNER, "folders", &team, VIEWER, "editor")
        .await;
    harness.share(OWNER, &board, VIEWER).await;

    let carried = harness.add(OWNER, &team, &board).await;
    let filed_again = harness.add(VIEWER, &team, &board).await;

    assert_eq!(carried.body, json!({ "carries": true }));
    assert_eq!(
        filed_again.body,
        json!({ "carries": true }),
        "a filing re-add never downgrades the stored entry"
    );
}

#[tokio::test]
async fn an_entry_the_owner_adds_again_becomes_theirs_and_outlasts_the_editor_s_grant() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness
        .grant(OWNER, "dashboards", &board, VIEWER, "editor")
        .await;
    let theirs = harness.folder_of(VIEWER, "Theirs").await;
    harness
        .grant(VIEWER, "folders", &theirs, STRANGER, "viewer")
        .await;
    harness
        .grant(VIEWER, "folders", &theirs, OWNER, "editor")
        .await;
    harness.add(VIEWER, &theirs, &board).await;

    let taken_over = harness.add(OWNER, &theirs, &board).await;
    harness.withdraw(OWNER, &board, VIEWER).await;

    assert_eq!(taken_over.body, json!({ "carries": true }));
    assert_eq!(
        harness.read(STRANGER, &board).await.body["access"],
        json!("viewer"),
        "the owner's entry carries whatever happens to the editor's grant"
    );
}

#[tokio::test]
async fn taking_out_a_dashboard_the_caller_cannot_see_is_not_found_and_leaves_its_entry() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness.share(OWNER, &board, VIEWER).await;
    let theirs = harness.folder_of(VIEWER, "Theirs").await;
    harness.add(VIEWER, &theirs, &board).await;
    harness.withdraw(OWNER, &board, VIEWER).await;

    let removed = harness
        .send(
            VIEWER,
            "DELETE",
            &format!("/v1/folders/{theirs}/dashboards/{board}"),
            None,
        )
        .await;
    harness.share(OWNER, &board, VIEWER).await;
    let contents = in_folder(&harness, VIEWER, &theirs).await;

    assert_eq!(removed.status, StatusCode::NOT_FOUND, "{}", removed.body);
    assert_eq!(names(&contents), ["Board"]);
}

#[tokio::test]
async fn an_orphaned_folder_is_not_handed_to_an_administrator_at_the_folder_cap() {
    let harness = Harness::new();
    let left = harness
        .definitions
        .create_folder(
            person(FORMER),
            crate::domain::folders::FolderName::parse("Left")
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    crate::domain::sharing::Grants::grant(
        harness.definitions.as_ref(),
        crate::domain::sharing::Shared::Folder(left.id),
        crate::domain::access::Grantee::Person(person(MEMBER)),
        crate::domain::access::Level::Viewer,
        person(FORMER),
    )
    .await
    .unwrap_or_else(|error| panic!("{error}"));
    for _ in 0..crate::domain::folders::MAX_FOLDERS {
        harness.folder_of(VIEWER, "Full").await;
    }

    let refused = harness
        .send(
            STRANGER,
            "POST",
            &format!("/v1/folders/{}/transfer", left.id),
            Some(json!({ "to": VIEWER })),
        )
        .await;

    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(
        refused.body.to_string().contains("too_many_folders"),
        "{}",
        refused.body
    );
}

#[tokio::test]
async fn a_malformed_folder_id_names_no_folder_on_any_folder_route() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    let routes = [
        (
            "PATCH",
            "/v1/folders/platform".to_owned(),
            Some(json!({ "name": "Platform" })),
        ),
        ("DELETE", "/v1/folders/platform".to_owned(), None),
        (
            "PUT",
            format!("/v1/folders/platform/dashboards/{board}"),
            None,
        ),
        (
            "DELETE",
            format!("/v1/folders/platform/dashboards/{board}"),
            None,
        ),
        ("GET", "/v1/folders/platform/grants".to_owned(), None),
        (
            "PUT",
            format!("/v1/folders/platform/grants/{VIEWER}"),
            Some(json!({ "level": "viewer" })),
        ),
        (
            "DELETE",
            format!("/v1/folders/platform/grants/{VIEWER}"),
            None,
        ),
        (
            "POST",
            "/v1/folders/platform/transfer".to_owned(),
            Some(json!({ "to": VIEWER })),
        ),
    ];

    for (method, path, body) in routes {
        let answer = harness.send(OWNER, method, &path, body).await;

        assert_eq!(
            answer.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            answer.body
        );
    }
}

fn visibility_of(listed: &super::Answer, id: &str) -> serde_json::Value {
    listed.body["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == json!(id)))
        .unwrap_or_else(|| panic!("{id} is listed: {}", listed.body))["visibility"]
        .clone()
}

#[tokio::test]
async fn a_listing_counts_the_grants_a_carrying_folder_brings_its_dashboards() {
    let harness = Harness::new();
    let to_people = harness.dashboard_of(OWNER, "To people").await;
    let to_everyone = harness.dashboard_of(OWNER, "To everyone").await;
    let team = harness.folder_of(OWNER, "Team").await;
    let org = harness.folder_of(OWNER, "Org").await;
    harness
        .grant(OWNER, "folders", &team, VIEWER, "viewer")
        .await;
    harness
        .grant(OWNER, "folders", &org, "everyone", "viewer")
        .await;
    harness.add(OWNER, &team, &to_people).await;
    harness.add(OWNER, &org, &to_everyone).await;

    let listed = harness.send(OWNER, "GET", "/v1/dashboards", None).await;

    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert_eq!(visibility_of(&listed, &to_people), json!("people"));
    assert_eq!(visibility_of(&listed, &to_everyone), json!("everyone"));
}

#[tokio::test]
async fn a_filing_entry_lends_its_folder_s_grants_to_no_one() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    let team = harness.folder_of(OWNER, "Team").await;
    harness
        .grant(OWNER, "folders", &team, VIEWER, "editor")
        .await;
    harness.add(OWNER, &team, &board).await;
    let theirs = harness.folder_of(VIEWER, "Theirs").await;
    harness
        .grant(VIEWER, "folders", &theirs, "everyone", "viewer")
        .await;
    let filed = harness.add(VIEWER, &theirs, &board).await;

    let listed = harness.send(OWNER, "GET", "/v1/dashboards", None).await;

    assert_eq!(filed.body, json!({ "carries": false }));
    assert_eq!(visibility_of(&listed, &board), json!("people"));
}

#[tokio::test]
async fn a_dashboard_carried_into_someone_else_s_folder_reaches_that_folder_s_owner() {
    let harness = Harness::new();
    let theirs = harness.folder_of(VIEWER, "Theirs").await;
    harness
        .grant(VIEWER, "folders", &theirs, OWNER, "editor")
        .await;
    let board = harness.dashboard_of(OWNER, "Board").await;
    let carried = harness.add(OWNER, &theirs, &board).await;
    let withdrawn = harness
        .send(
            VIEWER,
            "DELETE",
            &format!("/v1/folders/{theirs}/grants/{OWNER}"),
            None,
        )
        .await;

    let listed = harness.send(OWNER, "GET", "/v1/dashboards", None).await;

    assert_eq!(carried.body, json!({ "carries": true }));
    assert!(withdrawn.status.is_success(), "{}", withdrawn.body);
    assert_eq!(harness.read(VIEWER, &board).await.status, StatusCode::OK);
    assert_eq!(visibility_of(&listed, &board), json!("people"));
}
