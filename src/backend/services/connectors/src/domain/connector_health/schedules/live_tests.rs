use chrono::SubsecRound;
use sea_orm::{ConnectOptions, Database};
use sea_orm_migration::{MigrationTrait, SchemaManager};
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::migration::m20261110_000001_connector_schedules::Migration;

type R = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn health_enrichment_joins_saved_requests_with_live_jobs_by_instance() -> R {
    let url = std::env::var("INTEGRATION_TESTS_MARIADB_URL")?;
    let mut options = ConnectOptions::new(url);
    options.sqlx_logging(false);
    let db = Database::connect(options).await?;
    Migration.up(&SchemaManager::new(&db)).await?;
    let connector = format!("test-{}", &Uuid::now_v7().simple().to_string()[..16]);
    let now = Utc::now().trunc_subsecs(3);
    for (source, crons, revision) in [
        ("main", json!(["0 6 * * *"]), 3_u64),
        ("second", json!([]), 4),
    ] {
        db.execute_raw(Statement::from_sql_and_values(DbBackend::MySql,
            "INSERT INTO connector_schedules (connector, source_id, crons, revision, updated_by, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
            [connector.clone().into(), source.into(), crons.into(), revision.into(), Uuid::from_u128(1).into(), now.into()])).await?;
    }
    let mut response = super::tests::response();
    for row in &mut response.connectors {
        row.connector.clone_from(&connector);
    }
    let stub = crate::infra::cluster_stub::Stub::start(
        crate::infra::cluster_stub::Scenario::Accept,
        &connector,
        "main",
    )
    .await?;
    let mut object = stub.object().await;
    object["spec"]["schedules"] = json!(["0 6 * * *"]);
    object["metadata"]["annotations"]["insight.cyberfabric.com/schedule-origin"] = json!("api");
    stub.replace(object, crate::infra::cluster_stub::Scenario::Accept)
        .await;
    enrich(&db, &stub.cluster, &mut response).await?;
    let wire = serde_json::to_value(&response)?;
    let main = &wire["connectors"][0]["schedule_request"];
    assert_eq!(main["revision"], 3);
    assert_eq!(main["crons"], json!(["0 6 * * *"]));
    assert_eq!(main["last_observation"]["status"], "applied");
    assert!(main["last_observation"]["observed_at"].is_string());
    let second = &wire["connectors"][1]["schedule_request"];
    assert_eq!(second["revision"], 4);
    assert_eq!(second["crons"], json!([]));
    assert_eq!(second["last_observation"]["status"], "pending");
    assert_eq!(wire["connectors"][2]["schedule_request"]["revision"], 0);
    assert!(wire["connectors"][3]["schedule_request"].is_null());
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        "DELETE FROM connector_schedules WHERE connector = ?",
        [connector.into()],
    ))
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL and loopback sockets"]
async fn summary_preserves_verified_reset_and_refused_or_unverified_reset_status() -> R {
    use clickhouse::test::{Mock, handlers};

    use crate::api::AppState;
    use crate::domain::connector_schedules::{Change, Instance, Revision, save, validation::Crons};
    use crate::infra::{
        cluster_stub::{Scenario, Stub},
        identity::IdentityClient,
    };

    #[derive(Debug, serde::Serialize, clickhouse::Row)]
    struct Member {
        connector: String,
    }

    let mut options = ConnectOptions::new(std::env::var("INTEGRATION_TESTS_MARIADB_URL")?);
    options.sqlx_logging(false);
    let db = Database::connect(options).await?;
    Migration.up(&SchemaManager::new(&db)).await?;
    for (scenario, expected) in [
        (Scenario::Accept, "inherited"),
        (Scenario::Refuse(403), "applied"),
        (Scenario::Mismatch, "applied"),
        (Scenario::AnnotationAbsent, "applied"),
    ] {
        let connector = format!("test-{}", &Uuid::now_v7().simple().to_string()[..16]);
        let stub = Stub::start(Scenario::Accept, &connector, "main").await?;
        let history = Mock::new();
        let state = std::sync::Arc::new(AppState {
            db: db.clone(),
            ch: insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                history.url(),
                "example",
            )),
            identity: IdentityClient::new("")?,
            cluster: stub.cluster.clone(),
            schedule_slots: tokio::sync::Semaphore::new(4),
        });
        let instance = Instance::parse(&connector, "main")?;
        history.add(handlers::provide([Member {
            connector: connector.clone(),
        }]));
        let saved = save(
            &state,
            &instance,
            Revision::parse(Some("\"0\""))?,
            Change::Put(Crons::parse(&["0 6 * * *".to_owned()])?),
            Uuid::from_u128(1),
        )
        .await?;
        assert_eq!(saved.status, Some(ScheduleStatus::Applied));
        let mut object = stub.object().await;
        if scenario == Scenario::AnnotationAbsent {
            object["metadata"]["annotations"]
                .as_object_mut()
                .ok_or("annotations")?
                .remove("insight.cyberfabric.com/deployment-schedule");
        }
        stub.replace(object, scenario).await;
        history.add(handlers::provide([Member {
            connector: connector.clone(),
        }]));
        let reset = save(
            &state,
            &instance,
            Revision::parse(Some("\"1\""))?,
            Change::Delete,
            Uuid::from_u128(1),
        )
        .await;
        if expected == "inherited" {
            assert_eq!(serde_json::to_value(reset?.status)?, expected);
        } else {
            assert!(matches!(
                reset,
                Err(crate::domain::connector_schedules::Error::NotChanged(_)
                    | crate::domain::connector_schedules::Error::ResetUnavailable)
            ));
        }
        verify_summary(&db, &stub, &connector, expected).await?;
        db.execute_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            "DELETE FROM connector_schedules WHERE connector = ?",
            [connector.clone().into()],
        ))
        .await?;
    }
    Ok(())
}

async fn verify_summary(
    db: &sea_orm::DatabaseConnection,
    stub: &crate::infra::cluster_stub::Stub,
    connector: &str,
    expected: &str,
) -> R {
    let mut response = super::tests::response();
    for row in &mut response.connectors {
        connector.clone_into(&mut row.connector);
    }
    enrich(db, &stub.cluster, &mut response).await?;
    let wire = serde_json::to_value(response)?;
    assert_eq!(
        wire["connectors"][0]["schedule_request"]["last_observation"]["status"],
        expected
    );
    assert_eq!(
        wire["connectors"][0]["schedule_request"]["crons"],
        if expected == "inherited" {
            json!([])
        } else {
            json!(["0 6 * * *"])
        }
    );

    assert_eq!(
        stub.object().await["spec"]["schedules"],
        if expected == "inherited" {
            json!(["0 4 * * *"])
        } else {
            json!(["0 6 * * *"])
        }
    );
    Ok(())
}
