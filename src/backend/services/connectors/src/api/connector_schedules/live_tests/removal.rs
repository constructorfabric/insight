use axum::http::StatusCode;
use clickhouse::test::{Mock, handlers};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde_json::json;

use super::super::support::{CALLER, ConfiguredInstanceRecord, Harness, R};
use super::{send, send_request};
use crate::infra::db::entities::connector_schedules::Entity;

pub(super) async fn clear_after_removal(
    h: &Harness,
    db: &DatabaseConnection,
    sync_history: &Mock,
    connector: &str,
    uri: &str,
) -> R {
    let (status, json) = send((h, sync_history), "PUT", uri, Some("\"3\""), &["0 6 * * *"]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 4);
    sync_history.add(handlers::provide(Vec::<ConfiguredInstanceRecord>::new()));
    let (status, json) = send_request(h, "GET", uri, None, &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 4);
    assert_eq!(json["requested"]["crons"], json!(["0 6 * * *"]));
    assert!(json["live"].is_null());
    assert_eq!(json["status"], "pending");
    assert!(json["read_error"].is_null());
    assert_eq!(json["pending_reason"], "no_cronworkflow");

    for (method, revision, expected) in [
        ("PUT", "\"4\"", StatusCode::NOT_FOUND),
        ("DELETE", "\"3\"", StatusCode::PRECONDITION_FAILED),
        ("DELETE", "\"4\"", StatusCode::OK),
        ("DELETE", "\"5\"", StatusCode::OK),
    ] {
        sync_history.add(handlers::provide(Vec::<ConfiguredInstanceRecord>::new()));
        let (status, json) = send_request(h, method, uri, Some(revision), &["0 9 * * *"]).await?;
        assert_eq!(status, expected, "{method}, {revision}");
        if status == StatusCode::OK {
            assert_eq!(json["revision"], 5);
            assert_eq!(json["requested"]["crons"], json!([]));
            assert!(json["live"].is_null());
            assert_eq!(json["status"], "pending");
            assert!(json["read_error"].is_null());
            assert_eq!(json["pending_reason"], "no_cronworkflow");
        }
    }
    let row = Entity::find_by_id((connector.to_owned(), "main".to_owned()))
        .one(db)
        .await?
        .ok_or("cleared removed row missing")?;
    assert_eq!(row.revision, 5);
    assert_eq!(row.crons, json!([]));
    assert_eq!(row.updated_by, CALLER);
    Ok(())
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn a_pruned_instance_returns_at_revision_zero_and_saves_from_zero() -> R {
    let Some(db) = super::connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = super::identity();
    super::revisions::arrange_schedule(&db, &connector, 8, json!(["0 3 * * *"])).await?;
    Entity::delete_by_id((connector.clone(), "main".to_owned()))
        .exec(&db)
        .await?;

    let history = Mock::new();
    let h = super::super::support::harness(
        db.clone(),
        history.url(),
        super::super::support::context("user", &[])?,
        true,
    )
    .await?;
    history.add(handlers::provide(Vec::<ConfiguredInstanceRecord>::new()));
    let (status, _) = send_request(&h, "GET", &uri, None, &[]).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, response) = send((&h, &history), "GET", &uri, None, &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["revision"], 0);
    assert!(response["requested"].is_null());
    let (status, response) =
        send((&h, &history), "PUT", &uri, Some("\"0\""), &["0 7 * * *"]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["revision"], 1);
    assert_eq!(response["requested"]["crons"], json!(["0 7 * * *"]));
    super::clear(&db, &connector).await?;
    Ok(())
}
