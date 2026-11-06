use chrono::DateTime;
use sea_orm::{DatabaseBackend, DbErr, MockDatabase};
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::domain::connector_schedules::test_support::acknowledgement;

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
fn health_attaches_request_metadata_including_clears_and_inheritance() -> R {
    for (crons, expected) in [(json!(["0 3 * * *"]), "applied"), (json!([]), "inherited")] {
        let request = row(crons.clone());
        let ack = acknowledgement();
        let mut inherited = acknowledgement();
        inherited.source_id = "second".to_owned();
        inherited.applied_revision = Some(0);
        inherited.attempted_revision = 0;
        let mut response = health()?;
        response.checked_at = None;
        attach(
            &mut response,
            &[
                Snapshot {
                    request: Some(request),
                    acknowledgement: Some(ack),
                },
                Snapshot {
                    request: None,
                    acknowledgement: Some(inherited),
                },
            ],
        );
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
        assert_eq!(
            inherited_state["inherited"],
            json!({"crons": ["0 5 * * *"], "origin": "descriptor"})
        );
        assert_eq!(inherited_state["applied"]["revision"], 0);
        assert!(inherited_state["updated_at"].is_null());
        assert_eq!(
            response.connectors[2]
                .schedule_request
                .as_ref()
                .ok_or("missing inherited state")?
                .status,
            dto::ScheduleStatus::Inherited
        );
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
fn missing_acknowledgements_leave_observations_unknown_and_only_overrides_pending() -> R {
    for (request, expected) in [
        (None, "inherited"),
        (Some(row(json!([]))), "inherited"),
        (Some(row(json!(["0 3 * * *"]))), "pending"),
    ] {
        let revision = request.as_ref().map_or(0, |row| row.revision);
        let rows: Vec<_> = request
            .map(|request| Snapshot {
                request: Some(request),
                acknowledgement: None,
            })
            .into_iter()
            .collect();
        let mut response = health()?;
        attach(&mut response, &rows);
        let state = serde_json::to_value(&response.connectors[0].schedule_request)?;
        assert_eq!(state["status"], expected, "{revision}, {expected}");
        assert_eq!(state["revision"], revision, "{expected}");
        assert!(state["applied"].is_null(), "{expected}");
        assert!(state["inherited"].is_null(), "{expected}");
        assert!(state["error"].is_null(), "{expected}");
        assert_eq!(state["updated_at"].is_null(), revision == 0, "{expected}");
        assert_eq!(state["updated_by"].is_null(), revision == 0, "{expected}");
    }
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

#[test]
fn health_enriches_unconfigured_instances_but_not_rows_without_a_source() -> R {
    let mut response = health()?;
    response.connectors[0].source_id = None;
    response.connectors[1].configured = false;
    attach(&mut response, &[]);
    assert!(response.connectors[0].schedule_request.is_none());
    let state = response.connectors[1]
        .schedule_request
        .as_ref()
        .ok_or("unconfigured state missing")?;
    assert_eq!(state.status, dto::ScheduleStatus::Inherited);
    assert_eq!(state.revision, 0);
    Ok(())
}

#[test]
fn undecodable_stored_crons_only_hide_the_affected_request_state() -> R {
    let mut response = health()?;
    let mut valid = row(json!(["0 3 * * *"]));
    valid.source_id = "second".to_owned();
    let mut ack = acknowledgement();
    ack.source_id = "second".to_owned();
    attach(
        &mut response,
        &[
            Snapshot {
                request: Some(row(json!({}))),
                acknowledgement: None,
            },
            Snapshot {
                request: Some(valid),
                acknowledgement: Some(ack),
            },
        ],
    );
    assert!(response.connectors[0].schedule_request.is_none());
    let state = response.connectors[1]
        .schedule_request
        .as_ref()
        .ok_or("valid state missing")?;
    assert_eq!(state.crons, ["0 3 * * *"]);
    assert_eq!(state.revision, 2);
    assert_eq!(state.status, dto::ScheduleStatus::Applied);
    assert!(response.connectors[2].schedule_request.is_some());
    Ok(())
}
