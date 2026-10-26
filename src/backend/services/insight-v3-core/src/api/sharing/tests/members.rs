//! What a signed-in person without the administrator role reaches, and what
//! an editor may do.

use axum::http::StatusCode;
use serde_json::{Value, json};

use super::{Harness, MEMBER, OWNER, STRANGER, VIEWER};

/// A board drawing one widget over one metric, made by `owner`; answers the
/// board and the widget.
async fn drawn_board(harness: &Harness, owner: &str) -> (String, String) {
    let metric = harness
        .made(
            owner,
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
            owner,
            "widgets",
            "Commits",
            json!({ "type": "table", "metric": metric, "columns": ["total"] }),
        )
        .await;
    let board = harness
        .made(
            owner,
            "dashboards",
            "Board",
            json!({ "title": "Board", "items": [{ "heading": "Top" }, { "widget": widget }] }),
        )
        .await;

    (board, widget)
}

/// Every route only an administrator reaches, as a member asks it.
fn administrators_only(board: &str, widget: &str) -> Vec<(&'static str, String, Option<Value>)> {
    let definition = json!({ "name": "Mine", "body": { "title": "Mine", "widgets": [] } });

    vec![
        ("GET", "/v1/metrics".to_owned(), None),
        ("GET", "/v1/widgets".to_owned(), None),
        ("GET", format!("/v1/widgets/{widget}"), None),
        (
            "POST",
            "/v1/dashboards".to_owned(),
            Some(definition.clone()),
        ),
        ("PUT", format!("/v1/dashboards/{board}"), Some(definition)),
        ("DELETE", format!("/v1/dashboards/{board}"), None),
        ("GET", format!("/v1/dashboards/{board}/dependents"), None),
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
        ("GET", format!("/v1/dashboards/{board}/grants"), None),
        (
            "PUT",
            format!("/v1/dashboards/{board}/grants/{VIEWER}"),
            Some(json!({ "level": "viewer" })),
        ),
        (
            "DELETE",
            format!("/v1/dashboards/{board}/grants/{VIEWER}"),
            None,
        ),
        (
            "POST",
            format!("/v1/dashboards/{board}/widgets/{widget}/drilldown"),
            None,
        ),
        ("GET", "/v1/orphaned-dashboards".to_owned(), None),
        ("GET", "/v1/orphaned-folders".to_owned(), None),
        (
            "POST",
            format!("/v1/dashboards/{board}/transfer"),
            Some(json!({ "to": OWNER })),
        ),
    ]
}

#[tokio::test]
async fn a_member_is_refused_every_administrator_route_before_any_lookup() {
    let harness = Harness::new();
    let (board, widget) = drawn_board(&harness, OWNER).await;
    harness.share(OWNER, &board, MEMBER).await;
    let missing = "0192a1b2-0000-7000-8000-0000000000ff";

    for (method, path, body) in administrators_only(&board, &widget)
        .into_iter()
        .chain(administrators_only(missing, missing))
    {
        let refused = harness.send(MEMBER, method, &path, body).await;

        assert_eq!(
            refused.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            refused.body
        );
        assert!(
            refused.body.to_string().contains("admin role required"),
            "{method} {path}: {}",
            refused.body
        );
    }
}

#[tokio::test]
async fn a_member_opens_a_dashboard_shared_with_them_and_reads_its_widgets_through_it() {
    let harness = Harness::new();
    let (board, widget) = drawn_board(&harness, OWNER).await;
    harness
        .grant(OWNER, "dashboards", &board, MEMBER, "editor")
        .await;

    let read = harness.read(MEMBER, &board).await;
    let widgets = harness
        .send(
            MEMBER,
            "GET",
            &format!("/v1/dashboards/{board}/widgets"),
            None,
        )
        .await;
    let listed = harness.send(MEMBER, "GET", "/v1/dashboards", None).await;
    let pinned = harness
        .send(MEMBER, "PUT", &format!("/v1/pins/{board}"), None)
        .await;
    let tags = harness.send(MEMBER, "GET", "/v1/tags", None).await;

    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert_eq!(
        read.body["access"],
        json!("viewer"),
        "an editor grant gives a member only a viewer's access"
    );
    assert_eq!(widgets.status, StatusCode::OK, "{}", widgets.body);
    assert_eq!(widgets.body["items"][0]["id"], json!(widget));
    assert_eq!(widgets.body["items"][0]["name"], json!("Commits"));
    assert_eq!(widgets.body["items"][0]["body"]["type"], json!("table"));
    assert_eq!(listed.body["total"], 1, "{}", listed.body);
    assert_eq!(listed.body["items"][0]["access"], json!("viewer"));
    assert_eq!(pinned.status, StatusCode::NO_CONTENT, "{}", pinned.body);
    assert_eq!(tags.status, StatusCode::OK, "{}", tags.body);
}

#[tokio::test]
async fn a_widget_run_that_fails_reaches_a_member_as_a_generic_message() {
    let harness = Harness::new();
    let (board, widget) = drawn_board(&harness, OWNER).await;
    harness.share(OWNER, &board, MEMBER).await;
    let path = format!("/v1/dashboards/{board}/widgets/{widget}/run");

    harness.failing_read();
    let by_member = harness.send(MEMBER, "POST", &path, None).await;

    assert_eq!(
        by_member.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        by_member.body
    );
    assert_eq!(by_member.body["context"], json!({}), "{}", by_member.body);
    let said = by_member.body.to_string();
    for cause in [widget.as_str(), "metric", "commits", "clickhouse"] {
        assert!(!said.contains(cause), "{cause} reached the member: {said}");
    }
}

#[tokio::test]
async fn a_widget_the_dashboard_does_not_draw_is_not_run_through_it() {
    let harness = Harness::new();
    let (board, _) = drawn_board(&harness, OWNER).await;
    let (_, elsewhere) = drawn_board(&harness, OWNER).await;
    harness.share(OWNER, &board, MEMBER).await;

    let refused = harness
        .send(
            MEMBER,
            "POST",
            &format!("/v1/dashboards/{board}/widgets/{elsewhere}/run"),
            None,
        )
        .await;

    assert_eq!(refused.status, StatusCode::NOT_FOUND, "{}", refused.body);
}

#[tokio::test]
async fn a_member_never_sees_an_ownerless_dashboard_or_one_shared_with_nobody() {
    let harness = Harness::new();
    harness.ownerless("legacy").await;
    let private = harness.dashboard_of(OWNER, "Private").await;

    let listed = harness.send(MEMBER, "GET", "/v1/dashboards", None).await;
    let legacy = harness.read(MEMBER, "legacy").await;
    let unseen = harness.read(MEMBER, &private).await;
    let folders = harness.send(MEMBER, "GET", "/v1/folders", None).await;

    assert_eq!(listed.body["total"], 0, "{}", listed.body);
    for refused in [&legacy, &unseen] {
        assert_eq!(refused.status, StatusCode::NOT_FOUND, "{}", refused.body);
    }
    assert_eq!(folders.body, json!({ "folders": [], "unfiled": 0 }));
}

#[tokio::test]
async fn an_editor_edits_tags_and_shares_onward_but_does_not_delete() {
    let harness = Harness::new();
    let board = harness.dashboard_of(OWNER, "Board").await;
    harness
        .grant(OWNER, "dashboards", &board, STRANGER, "editor")
        .await;

    let edited = harness
        .send(
            STRANGER,
            "PUT",
            &format!("/v1/dashboards/{board}"),
            Some(json!({ "name": "Renamed", "body": { "title": "Renamed", "widgets": [] } })),
        )
        .await;
    let tagged = harness
        .send(
            STRANGER,
            "PUT",
            &format!("/v1/dashboards/{board}/tags"),
            Some(json!({ "tags": ["Delivery"] })),
        )
        .await;
    let shared = harness.share(STRANGER, &board, VIEWER).await;
    let deleted = harness
        .send(STRANGER, "DELETE", &format!("/v1/dashboards/{board}"), None)
        .await;
    let copied = harness
        .send(
            STRANGER,
            "POST",
            &format!("/v1/dashboards/{board}/duplicate"),
            Some(json!({ "name": "Copy" })),
        )
        .await;

    for done in [&edited, &tagged, &shared] {
        assert_eq!(done.status, StatusCode::NO_CONTENT, "{}", done.body);
    }
    assert_eq!(deleted.status, StatusCode::FORBIDDEN, "{}", deleted.body);
    assert!(
        deleted
            .body
            .to_string()
            .contains("only the owner can delete this dashboard"),
        "{}",
        deleted.body
    );
    assert_eq!(copied.status, StatusCode::CREATED, "{}", copied.body);
    let read = harness.read(OWNER, &board).await;
    assert_eq!(read.body["name"], json!("Renamed"));
    assert_eq!(read.body["tags"], json!(["Delivery"]));
    assert_eq!(
        harness.read(VIEWER, &board).await.body["access"],
        json!("viewer")
    );
    let copy = copied.body["id"].as_str().unwrap_or_default();
    assert_eq!(
        harness.read(STRANGER, copy).await.body["owner_person_id"],
        json!(STRANGER)
    );
}
