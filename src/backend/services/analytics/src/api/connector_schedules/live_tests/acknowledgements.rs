use sea_orm::{ActiveModelTrait, IntoActiveModel};

use super::*;
use crate::domain::connector_schedules::test_support::acknowledgement;
use crate::infra::db::entities::connector_schedule_acknowledgements as acknowledgements;

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn get_and_writes_return_acknowledged_facts_and_revision_status() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = identity();
    let sync_history = Mock::new();
    let h = harness(db.clone(), sync_history.url(), context("user", &[])?, true).await?;
    let target = (&h, &sync_history);
    assert_eq!(
        send(target, "PUT", &uri, Some("\"0\""), &["0 3 * * *"])
            .await?
            .0,
        StatusCode::OK
    );
    let mut ack = acknowledgement();
    ack.connector.clone_from(&connector);
    ack.applied_revision = Some(1);
    ack.attempted_revision = 1;
    ack.last_error = Some("synthetic readback failure".to_owned());
    ack.into_active_model().insert(&db).await?;

    for (method, revision, crons, next, expected) in [
        ("GET", None, vec![], 1, "error"),
        ("PUT", Some("\"1\""), vec!["0 6 * * *"], 2, "pending"),
        ("DELETE", Some("\"2\""), vec![], 3, "pending"),
    ] {
        let (status, response) = send(target, method, &uri, revision, &crons).await?;
        assert_eq!(status, StatusCode::OK, "{method}");
        assert_eq!(response["revision"], next, "{method}");
        assert_eq!(response["status"], expected, "{method}");
        assert_eq!(
            response["applied"],
            json!({
                "crons": ["0 3 * * *"], "timezone": "UTC", "origin": "api", "suspended": false,
                "revision": 1, "observed_at": "1970-01-01T00:00:00Z"
            }),
            "{method}"
        );
        assert_eq!(
            response["inherited"],
            json!({"crons": ["0 5 * * *"], "origin": "descriptor"}),
            "{method}"
        );
        assert_eq!(
            response["error"],
            json!({"attempted_revision": 1, "attempted_at": "1970-01-01T00:00:00Z", "message": "synthetic readback failure"}),
            "{method}"
        );
    }
    acknowledgements::Entity::delete_many()
        .filter(acknowledgements::Column::Connector.eq(&connector))
        .exec(&db)
        .await?;
    clear(&db, &connector).await
}
