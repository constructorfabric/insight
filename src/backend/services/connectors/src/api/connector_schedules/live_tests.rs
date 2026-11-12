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
use crate::migration::m20261110_000001_connector_schedules::Migration;

mod budgets;
mod cancellation;
mod cluster;
mod locks;
mod removal;
mod revisions;
mod storage;

async fn connect_or_skip() -> Result<Option<DatabaseConnection>, Box<dyn std::error::Error>> {
    let Ok(url) = std::env::var("INTEGRATION_TESTS_MARIADB_URL") else {
        eprintln!("skipping: INTEGRATION_TESTS_MARIADB_URL not set");
        return Ok(None);
    };
    let mut options = ConnectOptions::new(url);
    options.max_connections(10).sqlx_logging(false);
    let db = Database::connect(options).await?;
    Migration.up(&SchemaManager::new(&db)).await?;
    Ok(Some(db))
}

fn identity() -> (String, String) {
    let connector = format!("test-{}", &Uuid::now_v7().simple().to_string()[..16]);
    let uri = format!("/v1/connectors/{connector}/main/schedule");
    (connector, uri)
}

pub(super) async fn clear(db: &DatabaseConnection, connector: &str) -> R {
    Entity::delete_many()
        .filter(Column::Connector.eq(connector))
        .exec(db)
        .await?;
    Ok(())
}

async fn send_request(
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

async fn send(
    (h, sync_history): (&Harness, &Mock),
    method: &str,
    uri: &str,
    revision: Option<&str>,
    crons: &[&str],
) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
    sync_history.add(handlers::provide([configured_instance_record()]));
    send_request(h, method, uri, revision, crons).await
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn migration_and_schedule_lifecycle_preserve_revisions_and_caller() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = identity();
    let sync_history = Mock::new();
    let caller = context("user", &[])?;
    let h = harness(db.clone(), sync_history.url(), caller, true).await?;
    let target = (&h, &sync_history);

    let (status, json) = send(target, "GET", &uri, None, &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 0);
    assert!(json["requested"].is_null());
    let (status, json) = send(target, "DELETE", &uri, Some("\"0\""), &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 0);
    assert!(
        Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?
            .is_none()
    );

    let (status, json) = send(target, "PUT", &uri, Some("\"0\""), &[" 0\t3 * * * "]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 1);
    assert_eq!(json["requested"]["crons"], json!(["0 3 * * *"]));
    assert_eq!(json["requested"]["updated_by"], CALLER.to_string());
    assert_eq!(json["status"], "pending");
    assert_eq!(json["pending_reason"], "no_cronworkflow");
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

    let (status, json) = send(target, "PUT", &uri, Some("\"1\""), &["0 3 * * *"]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 1);
    let unchanged = Entity::find_by_id((connector.clone(), "main".to_owned()))
        .one(&db)
        .await?
        .ok_or("unchanged row missing")?;
    assert_eq!(unchanged, row);
    let (status, json) = send(target, "PUT", &uri, Some("\"1\""), &["0 4 * * *"]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 2);
    assert_eq!(json["status"], "pending");
    assert_eq!(json["pending_reason"], "no_cronworkflow");
    assert_eq!(
        send(target, "PUT", &uri, Some("\"1\""), &["0 5 * * *"])
            .await?
            .0,
        StatusCode::PRECONDITION_FAILED
    );
    assert_eq!(
        send(target, "PUT", &uri, Some("\"0\""), &["0 5 * * *"])
            .await?
            .0,
        StatusCode::PRECONDITION_FAILED
    );
    assert_eq!(
        send_request(&h, "PUT", &uri, None, &["0 5 * * *"]).await?.0,
        StatusCode::PRECONDITION_REQUIRED
    );
    assert_eq!(
        send_request(&h, "DELETE", &uri, None, &[]).await?.0,
        StatusCode::PRECONDITION_REQUIRED
    );

    let (status, json) = send(target, "DELETE", &uri, Some("\"2\""), &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 3);
    assert_eq!(json["requested"]["crons"], json!([]));
    let row = Entity::find_by_id((connector.clone(), "main".to_owned()))
        .one(&db)
        .await?
        .ok_or("delete removed row")?;
    assert_eq!(row.revision, 3);
    assert_eq!(row.crons, json!([]));
    let (status, json) = send(target, "DELETE", &uri, Some("\"3\""), &[]).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["revision"], 3);
    let unchanged = Entity::find_by_id((connector.clone(), "main".to_owned()))
        .one(&db)
        .await?
        .ok_or("cleared row missing")?;
    assert_eq!(unchanged, row);
    assert_eq!(
        send(target, "DELETE", &uri, Some("\"0\""), &[]).await?.0,
        StatusCode::PRECONDITION_FAILED
    );

    removal::clear_after_removal(&h, &db, &sync_history, &connector, &uri).await?;

    clear(&db, &connector).await
}
