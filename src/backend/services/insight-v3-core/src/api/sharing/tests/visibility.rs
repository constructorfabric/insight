//! Every route that can name a dashboard, walked as an administrator who
//! neither owns it nor was shared it, and as one it was shared with.

use axum::http::StatusCode;
use serde_json::{Value, json};

use super::{Answer, Harness, OWNER, STRANGER, VIEWER};

/// Each route that addresses one dashboard, with the body it sends.
fn addressed(id: &str) -> Vec<(&'static str, String, Option<Value>)> {
    let board = json!({ "title": "Taken", "widgets": [] });

    vec![
        ("GET", format!("/v1/dashboards/{id}"), None),
        (
            "PUT",
            format!("/v1/dashboards/{id}"),
            Some(json!({ "name": "Taken", "body": board })),
        ),
        ("DELETE", format!("/v1/dashboards/{id}"), None),
        (
            "POST",
            format!("/v1/dashboards/{id}/duplicate"),
            Some(json!({ "name": "Copy" })),
        ),
        (
            "PUT",
            format!("/v1/dashboards/{id}/folder"),
            Some(json!({ "folder": null })),
        ),
        (
            "PUT",
            format!("/v1/dashboards/{id}/tags"),
            Some(json!({ "tags": ["Alpha"] })),
        ),
        ("GET", format!("/v1/dashboards/{id}/dependents"), None),
        ("PUT", format!("/v1/pins/{id}"), None),
        ("DELETE", format!("/v1/pins/{id}"), None),
        ("GET", format!("/v1/dashboards/{id}/shares"), None),
        ("PUT", format!("/v1/dashboards/{id}/shares/{VIEWER}"), None),
        (
            "DELETE",
            format!("/v1/dashboards/{id}/shares/{VIEWER}"),
            None,
        ),
    ]
}

fn ids(listed: &Answer) -> Vec<String> {
    listed.body["items"]
        .as_array()
        .unwrap_or_else(|| panic!("a listing answers items: {}", listed.body))
        .iter()
        .map(|item| item["id"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test]
async fn every_route_answers_a_dashboard_the_caller_cannot_see_as_a_missing_one() {
    let harness = Harness::new();
    let private = harness.dashboard_of(OWNER, "Private").await;
    let missing = "0192a1b2-0000-7000-8000-0000000000ff";

    for ((method, path, body), (_, absent, absent_body)) in
        addressed(&private).into_iter().zip(addressed(missing))
    {
        let unseen = harness.send(STRANGER, method, &path, body).await;
        let gone = harness.send(STRANGER, method, &absent, absent_body).await;

        assert_eq!(
            unseen.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            unseen.body
        );
        assert_eq!(
            unseen.body.to_string().replace(&private, missing),
            gone.body.to_string(),
            "{method} {path} must answer as a missing dashboard does"
        );
    }
    assert_eq!(
        harness.read(OWNER, &private).await.body["name"],
        json!("Private")
    );
}

#[tokio::test]
async fn a_dashboard_the_caller_cannot_see_is_never_listed_counted_or_pinned() {
    let harness = Harness::new();
    let private = harness.dashboard_of(OWNER, "Private").await;
    harness
        .send(
            OWNER,
            "PUT",
            &format!("/v1/dashboards/{private}/tags"),
            Some(json!({ "tags": ["Secret"] })),
        )
        .await;
    let mine = harness.dashboard_of(STRANGER, "mine").await;
    harness.ownerless("legacy").await;

    let listed = harness.send(STRANGER, "GET", "/v1/dashboards", None).await;
    let searched = harness
        .send(STRANGER, "GET", "/v1/dashboards?q=Private", None)
        .await;
    let tagged = harness
        .send(STRANGER, "GET", "/v1/dashboards?tag=Secret", None)
        .await;
    let tags = harness.send(STRANGER, "GET", "/v1/tags", None).await;
    let folders = harness.send(STRANGER, "GET", "/v1/folders", None).await;

    assert_eq!(ids(&listed), vec!["legacy".to_owned(), mine]);
    assert_eq!(listed.body["total"], 2);
    assert_eq!(searched.body["total"], 0);
    assert_eq!(tagged.body["total"], 0);
    assert_eq!(tags.body["tags"], json!([]));
    assert_eq!(folders.body["unfiled"], 2);
}

#[tokio::test]
async fn a_viewer_opens_pins_and_files_a_shared_dashboard_but_changes_nothing() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness.share(OWNER, &board, VIEWER).await;
    let folder = harness
        .send(
            VIEWER,
            "POST",
            "/v1/folders",
            Some(json!({ "name": "Mine" })),
        )
        .await;

    let read = harness.read(VIEWER, &board).await;
    let pinned = harness
        .send(VIEWER, "PUT", &format!("/v1/pins/{board}"), None)
        .await;
    let filed = harness
        .send(
            VIEWER,
            "PUT",
            &format!("/v1/dashboards/{board}/folder"),
            Some(json!({ "folder": folder.body["id"] })),
        )
        .await;

    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert_eq!(read.body["access"], json!("viewer"));
    assert_eq!(read.body["owner_person_id"], json!(OWNER));
    assert_eq!(pinned.status, StatusCode::NO_CONTENT, "{}", pinned.body);
    assert_eq!(filed.status, StatusCode::NO_CONTENT, "{}", filed.body);
    let changes = [
        (
            "PUT",
            format!("/v1/dashboards/{board}"),
            Some(json!({ "name": "Mine now", "body": { "title": "x", "widgets": [] } })),
        ),
        ("DELETE", format!("/v1/dashboards/{board}"), None),
        (
            "POST",
            format!("/v1/dashboards/{board}/duplicate"),
            Some(json!({ "name": "Copy" })),
        ),
        (
            "PUT",
            format!("/v1/dashboards/{board}/tags"),
            Some(json!({ "tags": ["Alpha"] })),
        ),
    ];
    for (method, path, body) in changes {
        let refused = harness.send(VIEWER, method, &path, body).await;

        assert_eq!(
            refused.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            refused.body
        );
        assert!(
            refused
                .body
                .to_string()
                .contains("only the owner can change this dashboard"),
            "{method} {path}: {}",
            refused.body
        );
    }
    assert_eq!(
        harness.read(OWNER, &board).await.body["name"],
        json!("Board")
    );
}

#[tokio::test]
async fn a_listing_keeps_my_own_or_those_shared_with_me_and_otherwise_everything_i_see() {
    let harness = Harness::new();
    let mine = harness.dashboard_of(VIEWER, "mine").await;
    let shared = harness.dashboard_of(OWNER, "shared").await;
    harness.share(OWNER, &shared, VIEWER).await;
    harness.dashboard_of(OWNER, "Private").await;
    harness.ownerless("legacy").await;

    let everything = harness.send(VIEWER, "GET", "/v1/dashboards", None).await;
    let owned = harness
        .send(VIEWER, "GET", "/v1/dashboards?owned=me", None)
        .await;
    let shared_with_me = harness
        .send(VIEWER, "GET", "/v1/dashboards?owned=shared", None)
        .await;
    let unknown = harness
        .send(VIEWER, "GET", "/v1/dashboards?owned=anyone", None)
        .await;
    let metrics = harness
        .send(VIEWER, "GET", "/v1/metrics?owned=me", None)
        .await;

    assert_eq!(
        ids(&everything),
        vec!["legacy".to_owned(), mine.clone(), shared.clone()]
    );
    let accesses: Vec<Value> = everything.body["items"]
        .as_array()
        .map(|items| items.iter().map(|item| item["access"].clone()).collect())
        .unwrap_or_default();
    assert_eq!(
        accesses,
        [json!("ownerless"), json!("owner"), json!("viewer")]
    );
    assert_eq!(everything.body["items"][0]["owner_person_id"], Value::Null);
    assert_eq!(ids(&owned), vec![mine]);
    assert_eq!(ids(&shared_with_me), vec![shared]);
    for refused in [&unknown, &metrics] {
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
        assert_eq!(super::field_of(refused), json!("owned"));
    }
}

#[tokio::test]
async fn dependents_name_only_what_the_caller_sees_and_count_the_rest() {
    let harness = Harness::new();
    let metric = harness
        .made(
            STRANGER,
            "metrics",
            "Commit count",
            json!({
                "dataset": "commits",
                "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
            }),
        )
        .await;
    let widget = harness
        .made(
            STRANGER,
            "widgets",
            "Commits",
            json!({ "type": "table", "metric": metric, "columns": ["total"] }),
        )
        .await;
    let board = json!({ "title": "Board", "items": [{ "widget": widget }] });
    harness
        .made(OWNER, "dashboards", "Private", board.clone())
        .await;
    let seen = harness.made(STRANGER, "dashboards", "Seen", board).await;

    let dependents = harness
        .send(
            STRANGER,
            "GET",
            &format!("/v1/widgets/{widget}/dependents"),
            None,
        )
        .await;
    let refused = harness
        .send(STRANGER, "DELETE", &format!("/v1/widgets/{widget}"), None)
        .await;

    assert_eq!(
        dependents.body,
        json!({
            "holders": [{ "kind": "dashboards", "id": seen, "name": "Seen" }],
            "hidden": 1
        })
    );
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    let said = refused.body.to_string();
    assert!(
        said.contains("still in use by Seen and 1 dashboard you cannot see"),
        "{said}"
    );
    assert!(!said.contains("Private"), "{said}");
}

#[tokio::test]
async fn a_created_dashboard_is_the_callers_and_a_metric_is_nobodys() {
    let harness = Harness::new();

    let board = harness.dashboard_of(STRANGER, "Board").await;
    let metric = harness
        .made(
            STRANGER,
            "metrics",
            "Commit count",
            json!({
                "dataset": "commits",
                "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
            }),
        )
        .await;

    let read = harness.read(STRANGER, &board).await;
    assert_eq!(read.body["owner_person_id"], json!(STRANGER));
    assert_eq!(read.body["access"], json!("owner"));
    let by_another = harness
        .send(OWNER, "GET", &format!("/v1/metrics/{metric}"), None)
        .await;
    assert_eq!(by_another.status, StatusCode::OK, "{}", by_another.body);
    assert!(
        by_another.body.get("owner_person_id").is_none(),
        "{}",
        by_another.body
    );
}

#[tokio::test]
async fn an_ownerless_dashboard_is_changed_by_any_administrator() {
    let harness = Harness::new();
    harness.ownerless("legacy").await;

    let replaced = harness
        .send(
            STRANGER,
            "PUT",
            "/v1/dashboards/legacy",
            Some(json!({ "name": "Legacy", "body": { "title": "Legacy", "widgets": [] } })),
        )
        .await;
    let copied = harness
        .send(
            VIEWER,
            "POST",
            "/v1/dashboards/legacy/duplicate",
            Some(json!({ "name": "Mine" })),
        )
        .await;

    assert_eq!(replaced.status, StatusCode::NO_CONTENT, "{}", replaced.body);
    assert_eq!(copied.status, StatusCode::CREATED, "{}", copied.body);
    let copy = copied.body["id"].as_str().unwrap_or_default();
    assert_eq!(
        harness.read(VIEWER, copy).await.body["owner_person_id"],
        json!(VIEWER)
    );
    assert_eq!(
        harness.read(OWNER, copy).await.status,
        StatusCode::NOT_FOUND
    );
}
