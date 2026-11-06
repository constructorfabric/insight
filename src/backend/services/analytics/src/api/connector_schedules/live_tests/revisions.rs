use sea_orm::{ActiveModelTrait, ActiveValue::Set};

use super::*;
use crate::infra::db::entities::connector_schedules::{ActiveModel, Model};

async fn arrange_schedule(
    db: &DatabaseConnection,
    connector: &str,
    revision: u64,
    crons: Value,
) -> Result<Model, Box<dyn std::error::Error>> {
    Ok(ActiveModel {
        connector: Set(connector.to_owned()),
        source_id: Set("main".to_owned()),
        crons: Set(crons),
        revision: Set(revision),
        updated_by: Set(Uuid::from_u128(2)),
        updated_at: Set(chrono::DateTime::UNIX_EPOCH),
    }
    .insert(db)
    .await?)
}

async fn assert_unchanged(db: &DatabaseConnection, before: &Model) -> R {
    let after = Entity::find_by_id((before.connector.clone(), before.source_id.clone()))
        .one(db)
        .await?
        .ok_or("unchanged row missing")?;
    assert_eq!(after, *before);
    Ok(())
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn changed_crons_at_the_maximum_revision_never_update() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    for crons in [json!([" 0\t3 * * * "]), json!([])] {
        let (connector, uri) = identity();
        let before = arrange_schedule(&db, &connector, u64::MAX, crons).await?;
        let sync_history = Mock::new();
        let h = harness(db.clone(), sync_history.url(), context("user", &[])?, true).await?;
        let target = (&h, &sync_history);
        let revision = format!("\"{}\"", u64::MAX);
        let (method, unchanged): (&str, &[&str]) = if before.crons == json!([]) {
            ("DELETE", &[])
        } else {
            ("PUT", &["0 3 * * *"])
        };

        let (status, json) = send(target, method, &uri, Some(&revision), unchanged).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["revision"], u64::MAX);
        assert_eq!(json["requested"]["crons"], before.crons);
        assert_eq!(
            json["requested"]["updated_by"],
            before.updated_by.to_string()
        );
        assert_eq!(
            json["requested"]["updated_at"],
            serde_json::to_value(before.updated_at)?
        );
        assert_unchanged(&db, &before).await?;

        let (status, json) = send(target, "PUT", &uri, Some(&revision), &["0 6 * * *"]).await?;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json["context"]["violations"][0]["subject"], "schedule");
        assert_unchanged(&db, &before).await?;
        clear(&db, &connector).await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn stale_revisions_are_rejected_even_when_the_schedule_is_unchanged() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    for crons in [json!(["0 3 * * *"]), json!([])] {
        let (connector, uri) = identity();
        let before = arrange_schedule(&db, &connector, 1, crons).await?;
        let sync_history = Mock::new();
        let h = harness(db.clone(), sync_history.url(), context("user", &[])?, true).await?;
        let target = (&h, &sync_history);
        let (method, unchanged): (&str, &[&str]) = if before.crons == json!([]) {
            ("DELETE", &[])
        } else {
            ("PUT", &["0 3 * * *"])
        };

        for expected in [0, 2] {
            let revision = format!("\"{expected}\"");
            let (status, json) = send(target, method, &uri, Some(&revision), unchanged).await?;
            assert_eq!(
                status,
                StatusCode::PRECONDITION_FAILED,
                "{method}, {expected}"
            );
            assert_eq!(json["context"]["revision"], 1);
            assert_unchanged(&db, &before).await?;
        }
        clear(&db, &connector).await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn clearing_revision_zero_never_writes_and_refuses_an_existing_row() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = identity();
    let sync_history = Mock::new();
    let h = harness(db.clone(), sync_history.url(), context("user", &[])?, true).await?;
    let target = (&h, &sync_history);

    let (status, json) = send(target, "DELETE", &uri, Some("\"0\""), &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 0);
    assert!(json["requested"].is_null());
    assert!(
        Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?
            .is_none()
    );

    for crons in [json!(["0 3 * * *"]), json!([])] {
        let before = arrange_schedule(&db, &connector, 1, crons).await?;
        let (status, json) = send(target, "DELETE", &uri, Some("\"0\""), &[]).await?;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);
        assert_eq!(json["context"]["revision"], 1);
        assert_unchanged(&db, &before).await?;
        clear(&db, &connector).await?;
    }
    Ok(())
}
