use axum::http::StatusCode;
use clickhouse::test::{Mock, handlers};
use sea_orm::{
    ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, QueryFilter,
};
use sea_orm_migration::{MigrationTrait, SchemaManager};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::support::*;
use crate::infra::db::entities::connector_schedules::{Column, Entity};
use crate::migration::m20261103_000001_connector_schedules::Migration;

mod noop;
mod removal;

async fn connect_or_skip() -> Result<Option<DatabaseConnection>, Box<dyn std::error::Error>> {
    let Ok(url) = std::env::var("INTEGRATION_TESTS_MARIADB_URL") else {
        eprintln!("skipping: INTEGRATION_TESTS_MARIADB_URL not set");
        return Ok(None);
    };
    let mut options = ConnectOptions::new(url);
    options.max_connections(4).sqlx_logging(false);
    let db = Database::connect(options).await?;
    Migration.up(&SchemaManager::new(&db)).await?;
    Ok(Some(db))
}

fn identity() -> (String, String) {
    let connector = format!("test-{}", Uuid::now_v7());
    let uri = format!("/v1/connectors/{connector}/main/schedule");
    (connector, uri)
}

async fn clear(db: &DatabaseConnection, connector: &str) -> R {
    Entity::delete_many()
        .filter(Column::Connector.eq(connector))
        .exec(db)
        .await?;
    Ok(())
}

async fn send(
    h: &Harness,
    method: &str,
    uri: &str,
    revision: Option<&str>,
    crons: &[&str],
) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
    let response = h
        .app
        .clone()
        .oneshot(request(method, uri, revision, &json!({"crons": crons}))?)
        .await?;
    let status = response.status();
    if matches!(status, StatusCode::OK | StatusCode::PRECONDITION_FAILED) {
        let etag = response.headers().get("etag").cloned();
        let json = body(response).await?;
        assert_eq!(
            etag.ok_or("missing ETag")?.to_str()?,
            format!(
                "\"{}\"",
                if status == StatusCode::OK {
                    &json["revision"]
                } else {
                    &json["context"]["revision"]
                }
            )
        );
        return Ok((status, json));
    }
    Ok((status, body(response).await?))
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn migration_and_schedule_lifecycle_preserve_revisions_and_caller() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = identity();
    let sync_history = Mock::new();
    for _ in 0..12 {
        sync_history.add(handlers::provide([configured_instance_record()]));
    }
    let caller = context("user", &[])?;
    let h = harness(db.clone(), sync_history.url(), true, caller, true).await?;

    let (status, json) = send(&h, "GET", &uri, None, &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 0);
    assert!(json["requested"].is_null());
    let (status, json) = send(&h, "DELETE", &uri, Some("\"0\""), &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 0);
    assert!(
        Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?
            .is_none()
    );

    let (status, json) = send(&h, "PUT", &uri, Some("\"0\""), &[" 0\t3 * * * "]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 1);
    assert_eq!(json["requested"]["crons"], json!(["0 3 * * *"]));
    assert_eq!(json["requested"]["updated_by"], CALLER.to_string());
    assert_eq!(json["status"], "applied");
    let row = Entity::find_by_id((connector.clone(), "main".to_owned()))
        .one(&db)
        .await?
        .ok_or("created row missing")?;
    assert_eq!(row.updated_at.timestamp_subsec_nanos() % 1_000_000, 0);
    assert_eq!(row.updated_by, CALLER);
    assert_eq!(
        json["requested"]["updated_at"],
        serde_json::to_value(row.updated_at)?
    );

    let (status, json) = send(&h, "PUT", &uri, Some("\"1\""), &["0 3 * * *"]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 1);
    let unchanged = Entity::find_by_id((connector.clone(), "main".to_owned()))
        .one(&db)
        .await?
        .ok_or("unchanged row missing")?;
    assert_eq!(unchanged, row);
    let (status, json) = send(&h, "PUT", &uri, Some("\"1\""), &["0 4 * * *"]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 2);
    assert_eq!(json["status"], "pending");
    assert_eq!(
        send(&h, "PUT", &uri, Some("\"1\""), &["0 5 * * *"])
            .await?
            .0,
        StatusCode::PRECONDITION_FAILED
    );
    assert_eq!(
        send(&h, "PUT", &uri, Some("\"0\""), &["0 5 * * *"])
            .await?
            .0,
        StatusCode::PRECONDITION_FAILED
    );
    assert_eq!(
        send(&h, "PUT", &uri, None, &["0 5 * * *"]).await?.0,
        StatusCode::PRECONDITION_REQUIRED
    );
    assert_eq!(
        send(&h, "DELETE", &uri, None, &[]).await?.0,
        StatusCode::PRECONDITION_REQUIRED
    );

    let (status, json) = send(&h, "DELETE", &uri, Some("\"2\""), &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 3);
    assert_eq!(json["requested"]["crons"], json!([]));
    let row = Entity::find_by_id((connector.clone(), "main".to_owned()))
        .one(&db)
        .await?
        .ok_or("delete removed row")?;
    assert_eq!(row.revision, 3);
    assert_eq!(row.crons, json!([]));
    let (status, json) = send(&h, "DELETE", &uri, Some("\"3\""), &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 3);
    let unchanged = Entity::find_by_id((connector.clone(), "main".to_owned()))
        .one(&db)
        .await?
        .ok_or("cleared row missing")?;
    assert_eq!(unchanged, row);
    assert_eq!(
        send(&h, "DELETE", &uri, Some("\"0\""), &[]).await?.0,
        StatusCode::PRECONDITION_FAILED
    );

    removal::clear_after_removal(&h, &db, &sync_history, &connector, &uri).await?;

    clear(&db, &connector).await
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn concurrent_puts_have_exactly_one_winner_for_creation_and_update() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = identity();
    let sync_history = Mock::new();
    for _ in 0..4 {
        sync_history.add(handlers::provide([configured_instance_record()]));
    }
    let h = harness(
        db.clone(),
        sync_history.url(),
        true,
        context("user", &[])?,
        true,
    )
    .await?;
    for expected in [0, 1] {
        let revision = format!("\"{expected}\"");
        let first_cron = format!("0 {} * * *", 3 + expected * 6);
        let second_cron = format!("0 {} * * *", 6 + expected * 6);
        let first_crons = [first_cron.as_str()];
        let second_crons = [second_cron.as_str()];
        let first = send(&h, "PUT", &uri, Some(&revision), &first_crons);
        let second = send(&h, "PUT", &uri, Some(&revision), &second_crons);
        let (first, second) = tokio::join!(first, second);
        let mut statuses = [first?.0.as_u16(), second?.0.as_u16()];
        statuses.sort_unstable();
        assert_eq!(statuses, [200, 412], "revision {expected}");
        let row = Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?
            .ok_or("winner row missing")?;
        assert_eq!(row.revision, expected + 1);
    }
    clear(&db, &connector).await
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn reconcile_lists_only_nonempty_rows_in_key_order_with_editing_off() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, _) = identity();
    let sync_history = Mock::new();
    for _ in 0..4 {
        sync_history.add(handlers::provide([configured_instance_record()]));
    }
    let h = harness(
        db.clone(),
        sync_history.url(),
        true,
        context("user", &[])?,
        true,
    )
    .await?;
    for source in ["z-last", "a-first", "cleared"] {
        let uri = format!("/v1/connectors/{connector}/{source}/schedule");
        assert_eq!(
            send(&h, "PUT", &uri, Some("\"0\""), &["0 3 * * *"])
                .await?
                .0,
            StatusCode::OK
        );
    }
    let cleared = format!("/v1/connectors/{connector}/cleared/schedule");
    assert_eq!(
        send(&h, "DELETE", &cleared, Some("\"1\""), &[]).await?.0,
        StatusCode::OK
    );
    let reader = harness(
        db.clone(),
        "http://127.0.0.1:1",
        false,
        context("service", &["connector-schedules-reader"])?,
        false,
    )
    .await?;
    let response = reader
        .app
        .clone()
        .oneshot(request(
            "GET",
            "/v1/connector-schedules",
            None,
            &json!(null),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let json = body(response).await?;
    assert_eq!(json["version"], 1);
    let rows = json["schedules"].as_array().ok_or("schedules missing")?;
    let keys = rows
        .iter()
        .map(|row| {
            (
                row["connector"].as_str().unwrap_or_default(),
                row["source_id"].as_str().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "{keys:?}");
    assert!(rows.iter().all(|row| {
        row["crons"]
            .as_array()
            .is_some_and(|crons| !crons.is_empty())
    }));
    let own = rows
        .iter()
        .filter(|row| row["connector"] == connector)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        own,
        json!([
            {"connector": connector, "source_id": "a-first", "crons": ["0 3 * * *"], "revision": 1},
            {"connector": connector, "source_id": "z-last", "crons": ["0 3 * * *"], "revision": 1}
        ])
        .as_array()
        .ok_or("expected rows missing")?
        .clone()
    );
    clear(&db, &connector).await
}

async fn race_at_revision_one(first: (&str, &[&str]), second: (&str, &[&str])) -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = identity();
    let sync_history = Mock::new();
    for _ in 0..3 {
        sync_history.add(handlers::provide([configured_instance_record()]));
    }
    let h = harness(
        db.clone(),
        sync_history.url(),
        true,
        context("user", &[])?,
        true,
    )
    .await?;
    assert_eq!(
        send(&h, "PUT", &uri, Some("\"0\""), &["0 6 * * *"])
            .await?
            .0,
        StatusCode::OK
    );
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let first = crate::domain::connector_schedules::AFTER_WRITE_READ.scope(
        barrier.clone(),
        send(&h, first.0, &uri, Some("\"1\""), first.1),
    );
    let second = crate::domain::connector_schedules::AFTER_WRITE_READ
        .scope(barrier, send(&h, second.0, &uri, Some("\"1\""), second.1));
    let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::join!(first, second)
    })
    .await?;
    let first = first?;
    let second = second?;
    let row = Entity::find_by_id((connector.clone(), "main".to_owned()))
        .one(&db)
        .await?
        .ok_or("winner missing")?;
    clear(&db, &connector).await?;
    assert_eq!(row.revision, 2);
    let mut statuses = [first.0.as_u16(), second.0.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 412]);
    let (winner, loser) = if first.0 == StatusCode::OK {
        (&first.1, &second.1)
    } else {
        (&second.1, &first.1)
    };
    assert_eq!(winner["revision"], 2);
    assert_eq!(loser["context"]["revision"], winner["revision"]);
    assert_eq!(row.crons, winner["requested"]["crons"]);
    Ok(())
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn put_and_delete_at_one_revision_have_one_winner_and_the_same_etag() -> R {
    race_at_revision_one(("PUT", &["0 9 * * *"]), ("DELETE", &[])).await
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn two_deletes_bump_the_revision_only_once() -> R {
    race_at_revision_one(("DELETE", &[]), ("DELETE", &[])).await
}
