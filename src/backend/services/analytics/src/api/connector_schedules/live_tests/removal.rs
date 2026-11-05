use axum::http::StatusCode;
use clickhouse::test::{Mock, handlers};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde_json::json;

use super::super::support::{
    CALLER, ConfiguredInstanceRecord, Harness, R, configured_instance_record,
};
use super::send;
use crate::domain::connector_schedules;
use crate::infra::db::entities::connector_schedules::Entity;

pub(super) async fn clear_after_removal(
    h: &Harness,
    db: &DatabaseConnection,
    sync_history: &Mock,
    connector: &str,
    uri: &str,
) -> R {
    sync_history.add(handlers::provide([configured_instance_record()]));
    let (status, json) = send(h, "PUT", uri, Some("\"3\""), &["0 6 * * *"]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 4);
    sync_history.add(handlers::provide(Vec::<ConfiguredInstanceRecord>::new()));
    sync_history.add(handlers::provide(Vec::<ConfiguredInstanceRecord>::new()));
    let (status, json) = send(h, "GET", uri, None, &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 4);
    assert_eq!(json["requested"]["crons"], json!(["0 6 * * *"]));
    assert!(json["applied"].is_null());
    assert_eq!(json["status"], "pending");
    assert!(
        connector_schedules::list(db)
            .await?
            .schedules
            .iter()
            .any(|row| row.connector == connector)
    );

    for (method, revision, expected) in [
        ("PUT", "\"4\"", StatusCode::NOT_FOUND),
        ("DELETE", "\"3\"", StatusCode::PRECONDITION_FAILED),
        ("DELETE", "\"4\"", StatusCode::OK),
        ("DELETE", "\"5\"", StatusCode::OK),
    ] {
        sync_history.add(handlers::provide(Vec::<ConfiguredInstanceRecord>::new()));
        let (status, json) = send(h, method, uri, Some(revision), &["0 9 * * *"]).await?;
        assert_eq!(status, expected, "{method}, {revision}");
        if status == StatusCode::OK {
            assert_eq!(json["revision"], 5);
            assert_eq!(json["requested"]["crons"], json!([]));
            assert!(json["applied"].is_null());
            assert_eq!(json["status"], "inherited");
        }
    }
    let row = Entity::find_by_id((connector.to_owned(), "main".to_owned()))
        .one(db)
        .await?
        .ok_or("cleared removed row missing")?;
    assert_eq!(row.revision, 5);
    assert_eq!(row.crons, json!([]));
    assert_eq!(row.updated_by, CALLER);
    assert!(
        !connector_schedules::list(db)
            .await?
            .schedules
            .iter()
            .any(|row| row.connector == connector)
    );
    Ok(())
}
