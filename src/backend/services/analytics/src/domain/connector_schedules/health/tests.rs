use chrono::DateTime;
use sea_orm::{DatabaseBackend, DbErr, MockDatabase};
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::domain::connector_schedules::test_support::{acknowledgement, snapshot_row};

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

#[tokio::test]
async fn health_batches_requests_and_acknowledgements_including_clears_and_inheritance() -> R {
    for (crons, expected) in [(json!(["0 3 * * *"]), "applied"), (json!([]), "inherited")] {
        let request = row(crons.clone());
        let ack = acknowledgement();
        let mut inherited = acknowledgement();
        inherited.source_id = "second".to_owned();
        inherited.applied_revision = Some(0);
        inherited.attempted_revision = 0;
        let db = MockDatabase::new(DatabaseBackend::MySql)
            .append_query_results([vec![
                snapshot_row(Some(&request), Some(&ack)),
                snapshot_row(None, Some(&inherited)),
            ]])
            .into_connection();
        let mut response = health()?;
        response.checked_at = None;
        enrich_health(&db, &mut response).await;
        let state = serde_json::to_value(&response.connectors[0].schedule_request)?;
        assert_eq!(state["crons"], crons);
        assert_eq!(state["status"], expected);
        assert_eq!(state["updated_by"], Uuid::from_u128(1).to_string());
        assert_eq!(state["updated_at"], "1970-01-01T00:00:00Z");
        assert_eq!(state["revision"], 2);
        assert_eq!(state["applied"]["revision"], 2);
        assert_eq!(state["inherited"]["origin"], "descriptor");
        let inherited_state = serde_json::to_value(&response.connectors[1].schedule_request)?;
        assert_eq!(inherited_state["status"], "inherited");
        assert!(inherited_state["updated_at"].is_null());
        assert!(response.connectors[2].schedule_request.is_none());
        let log = db.into_transaction_log();
        assert_eq!(log.len(), 1);
        let sql = format!("{log:?}");
        assert!(sql.contains("connector_schedule_acknowledgements"), "{sql}");
        assert!(!sql.contains("JSON_LENGTH"), "{sql}");
    }
    Ok(())
}

#[test]
fn health_uses_acknowledgement_errors_instead_of_the_configured_schedule() -> R {
    let request = row(json!(["0 3 * * *"]));
    for (attempted, expected) in [(2, "error"), (1, "pending")] {
        let mut ack = acknowledgement();
        ack.applied_revision = Some(1);
        ack.attempted_revision = attempted;
        ack.last_error = Some("synthetic readback failure".to_owned());
        let mut response = health()?;
        attach(
            &mut response,
            &[Snapshot {
                request: Some(request.clone()),
                acknowledgement: Some(ack),
            }],
        );
        let state = serde_json::to_value(&response.connectors[0].schedule_request)?;
        assert_eq!(state["status"], expected);
        assert_eq!(state["error"]["attempted_revision"], attempted);
        assert_eq!(state["error"]["message"], "synthetic readback failure");
    }
    Ok(())
}

#[test]
fn missing_acknowledgements_leave_observations_unknown_and_requests_pending() -> R {
    let mut response = health()?;
    attach(
        &mut response,
        &[Snapshot {
            request: Some(row(json!([]))),
            acknowledgement: None,
        }],
    );
    let state = serde_json::to_value(&response.connectors[0].schedule_request)?;
    assert_eq!(state["status"], "pending");
    assert!(state["applied"].is_null());
    assert!(state["inherited"].is_null());
    assert!(state["error"].is_null());
    Ok(())
}

#[tokio::test]
async fn maria_failure_keeps_every_request_null_and_health_available() -> R {
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_errors([DbErr::Custom("synthetic failure".to_owned())])
        .into_connection();
    let mut response = health()?;
    let before = serde_json::to_value(&response)?;
    enrich_health(&db, &mut response).await;
    assert!(response.history_available);
    assert!(
        response
            .connectors
            .iter()
            .all(|row| row.schedule_request.is_none())
    );
    assert_eq!(serde_json::to_value(&response)?, before);
    Ok(())
}
