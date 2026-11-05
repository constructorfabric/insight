use sea_orm::{DatabaseBackend, DbErr, MockDatabase};
use serde_json::json;
use uuid::Uuid;

use super::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn health() -> Result<ConnectorHealthResponse, serde_json::Error> {
    serde_json::from_value(json!({
        "as_of": "2026-01-15T13:00:00Z", "checked_at": "2026-01-15T12:45:00Z",
        "history_available": true,
        "connectors": [
            {"connector": "example-tracker", "source_id": "main", "configured": true,
             "schedule": {"crons": ["0 3 * * *"], "timezone": "UTC", "origin": "api", "suspended": false}},
            {"connector": "example-tracker", "source_id": "second", "configured": true},
            {"connector": "example-directory", "source_id": "main", "configured": true}
        ]
    }))
}

fn row(crons: serde_json::Value) -> Model {
    Model {
        connector: "example-tracker".to_owned(),
        source_id: "main".to_owned(),
        crons,
        revision: 2,
        updated_by: Uuid::from_u128(1),
        updated_at: DateTime::UNIX_EPOCH,
    }
}

#[test]
fn requests_match_both_instance_keys_and_keep_clear_metadata() -> R {
    for (crons, expected) in [(json!(["0 3 * * *"]), "applied"), (json!([]), "pending")] {
        let mut response = health()?;
        attach(&mut response, &[row(crons.clone())], &[]);
        let state = serde_json::to_value(&response.connectors[0].schedule_request)?;
        assert_eq!(state["crons"], crons);
        assert_eq!(state["status"], expected);
        assert_eq!(state["updated_by"], Uuid::from_u128(1).to_string());
        assert_eq!(state["updated_at"], "1970-01-01T00:00:00Z");
        for other in &response.connectors[1..] {
            assert!(other.schedule_request.is_none());
        }
    }
    Ok(())
}

#[test]
fn inherited_requests_are_absent_and_failed_passes_use_the_shared_rule() -> R {
    let mut response = health()?;
    let mut cleared = row(json!([]));
    cleared.source_id = "second".to_owned();
    let completed_passes = [
        DateTime::UNIX_EPOCH + chrono::Duration::minutes(30),
        DateTime::UNIX_EPOCH + chrono::Duration::minutes(15),
    ];
    attach(
        &mut response,
        &[row(json!(["0 6 * * *"])), cleared],
        &completed_passes,
    );
    let state = response.connectors[0]
        .schedule_request
        .as_ref()
        .ok_or("missing request")?;
    assert_eq!(state.status, ScheduleStatus::NotApplied);
    assert_eq!(state.not_applied_since, Some(completed_passes[0]));
    assert!(response.connectors[1].schedule_request.is_none());
    Ok(())
}

#[tokio::test]
async fn maria_failure_keeps_every_request_null_and_health_available() -> R {
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_errors([DbErr::Custom("synthetic failure".to_owned())])
        .into_connection();
    let ch = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
        "http://127.0.0.1:1",
        "analytics",
    ));
    let mut response = health()?;
    let before = serde_json::to_value(&response)?;
    let config = ConnectorSchedulesConfig {
        enabled: true,
        ..ConnectorSchedulesConfig::default()
    };
    enrich_health(&db, &ch, &config, &mut response).await;
    assert!(response.history_available);
    assert_eq!(serde_json::to_value(&response)?, before);
    assert!(
        response
            .connectors
            .iter()
            .all(|row| row.schedule_request.is_none())
    );
    assert_eq!(db.into_transaction_log().len(), 1);
    Ok(())
}

#[tokio::test]
async fn editing_off_reads_no_requests() -> R {
    let db = MockDatabase::new(DatabaseBackend::MySql).into_connection();
    let ch = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
        "http://127.0.0.1:1",
        "analytics",
    ));
    let mut response = health()?;
    enrich_health(
        &db,
        &ch,
        &ConnectorSchedulesConfig::default(),
        &mut response,
    )
    .await;
    assert!(
        response
            .connectors
            .iter()
            .all(|row| row.schedule_request.is_none())
    );
    assert!(db.into_transaction_log().is_empty());
    Ok(())
}
