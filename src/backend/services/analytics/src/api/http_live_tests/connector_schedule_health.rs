use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter};
use sea_orm_migration::{MigrationTrait, SchemaManager};

use super::*;
use crate::domain::connector_schedules::test_support::acknowledgement;
use crate::infra::db::entities::connector_schedule_acknowledgements as acknowledgements;
use crate::migration::m20261103_000001_connector_schedules::Migration;

pub(crate) async fn assert_health_enrichment(ch: &insight_clickhouse::Client) -> TestResult {
    let Ok(url) = std::env::var(ENV_VAR) else {
        eprintln!("skipping health enrichment: {ENV_VAR} not set");
        return Ok(());
    };
    let db = Database::connect(url).await?;
    Migration.up(&SchemaManager::new(&db)).await?;
    let mut ack = acknowledgement();
    ack.applied_revision = Some(0);
    ack.attempted_revision = 0;
    ack.into_active_model().insert(&db).await?;
    let identity = spawn_identity_with_roles(&[], true).await?;
    let app = app_with_identity_and_ch(db.clone(), Uuid::now_v7(), identity, ch.clone());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/connector-health")
                .header("authorization", "Bearer synthetic-token")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    let response: Value = serde_json::from_slice(&bytes)?;
    let row = response["connectors"]
        .as_array()
        .ok_or("connectors missing")?
        .iter()
        .find(|row| row["connector"] == "example-tracker" && row["source_id"] == "main")
        .ok_or("configured health row missing")?;
    assert_eq!(row["configured"], true);
    let state = &row["schedule_request"];
    assert_eq!(state["revision"], 0);
    assert_eq!(state["status"], "inherited");
    assert_eq!(state["crons"], json!([]));
    assert_eq!(
        state["applied"],
        json!({
            "crons": ["0 3 * * *"], "timezone": "UTC", "origin": "api", "suspended": false,
            "revision": 0, "observed_at": "1970-01-01T00:00:00Z"
        })
    );
    assert_eq!(
        state["inherited"],
        json!({"crons": ["0 5 * * *"], "origin": "descriptor"})
    );
    assert_eq!(state.get("updated_at"), Some(&Value::Null));
    assert_eq!(state.get("updated_by"), Some(&Value::Null));
    assert_eq!(state.get("error"), Some(&Value::Null));
    acknowledgements::Entity::delete_many()
        .filter(acknowledgements::Column::Connector.eq("example-tracker"))
        .filter(acknowledgements::Column::SourceId.eq("main"))
        .exec(&db)
        .await?;
    Ok(())
}
