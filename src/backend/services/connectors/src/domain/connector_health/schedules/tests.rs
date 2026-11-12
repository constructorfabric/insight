use sea_orm::{DatabaseBackend, DbErr, MockDatabase};
use serde_json::json;

use super::*;
use crate::domain::connector_health::model::{ConnectorHealth, LedgerFacts};
use crate::infra::cluster_stub::{Scenario, Stub};

type R = Result<(), Box<dyn std::error::Error>>;

fn stored(crons: serde_json::Value) -> StoredRequest {
    StoredRequest { revision: 3, crons }
}

pub(super) fn response() -> ConnectorHealthResponse {
    let mut response =
        ConnectorHealthResponse::from_facts(LedgerFacts::default(), DateTime::UNIX_EPOCH);
    for source in [Some("main"), Some("second"), Some("unrequested"), None] {
        response.connectors.push(ConnectorHealth {
            connector: "example-tracker".to_owned(),
            source_id: source.map(str::to_owned),
            tenant_id: source.map(|_| "example-tenant".to_owned()),
            configured: true,
            schedule: None,
            schedule_request: None,
            last_sync: None,
        });
    }
    response
}

#[tokio::test]
async fn unconfigured_rows_have_no_schedule_or_request_even_with_a_leftover_sync_job() -> R {
    for (scenario, case) in [
        (Scenario::Missing, "missing"),
        (Scenario::Accept, "leftover"),
    ] {
        let stub = Stub::start(scenario, "example-tracker", "main").await?;
        let db = MockDatabase::new(DatabaseBackend::MySql)
            .append_query_results([
                Vec::<std::collections::BTreeMap<String, sea_orm::Value>>::new(),
            ])
            .into_connection();
        let mut response = response();
        response.connectors[0].configured = false;

        enrich(&db, &stub.cluster, &mut response).await?;

        let wire = serde_json::to_value(response)?;
        assert!(
            wire["connectors"][0]["schedule_request"].is_null(),
            "{case}"
        );
        assert!(wire["connectors"][0]["schedule"].is_null(), "{case}");
        assert_eq!(
            wire["connectors"][1]["schedule_request"]["last_observation"]["status"], "pending",
            "{case}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn summary_uses_live_jobs_after_manual_changes_and_lists_once_per_request() -> R {
    let stub = Stub::start(Scenario::Accept, "example-tracker", "main").await?;
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([
            Vec::<std::collections::BTreeMap<String, sea_orm::Value>>::new(),
            Vec::new(),
        ])
        .into_connection();
    let mut first = response();
    enrich(&db, &stub.cluster, &mut first).await?;
    assert_eq!(
        first.connectors[0]
            .schedule_request
            .as_ref()
            .ok_or("request")?
            .last_observation
            .as_ref()
            .ok_or("status")?
            .status,
        ScheduleStatus::Inherited
    );
    assert_eq!(
        first.connectors[0].schedule.as_ref().ok_or("live")?.crons,
        ["0 4 * * *"]
    );
    assert_eq!(stub.lists().await, 1);
    let mut manual = stub.object().await;
    manual["spec"]["schedules"] = json!(["0 9 * * *"]);
    stub.replace(manual, Scenario::Accept).await;
    let mut second = response();
    enrich(&db, &stub.cluster, &mut second).await?;
    assert_eq!(
        second.connectors[0]
            .schedule_request
            .as_ref()
            .ok_or("request")?
            .last_observation
            .as_ref()
            .ok_or("status")?
            .status,
        ScheduleStatus::Pending
    );
    assert_eq!(
        second.connectors[0].schedule.as_ref().ok_or("live")?.crons,
        ["0 9 * * *"]
    );
    assert_eq!(stub.lists().await, 2);
    assert!(stub.patches().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn summary_status_compares_current_jobs_and_stored_requests() -> R {
    let stub = Stub::start(Scenario::Accept, "example-tracker", "main").await?;
    for (crons, live_cron, timezone, inherited, expected) in [
        (
            json!(["0 6 * * *"]),
            "0 6 * * *",
            "UTC",
            true,
            ScheduleStatus::Applied,
        ),
        (
            json!(["0 6 * * *"]),
            "0 9 * * *",
            "UTC",
            true,
            ScheduleStatus::Pending,
        ),
        (
            json!(["0 6 * * *"]),
            "0 9 * * *",
            "UTC",
            true,
            ScheduleStatus::Pending,
        ),
        (
            json!(["0 6 * * *"]),
            "0 6 * * *",
            "Europe/London",
            true,
            ScheduleStatus::Pending,
        ),
        (
            json!([]),
            "0 4 * * *",
            "UTC",
            true,
            ScheduleStatus::Inherited,
        ),
        (json!([]), "0 6 * * *", "UTC", true, ScheduleStatus::Pending),
        (
            json!([]),
            "0 4 * * *",
            "UTC",
            false,
            ScheduleStatus::Pending,
        ),
    ] {
        let mut object = stub.object().await;
        object["spec"]["schedules"] = json!([live_cron]);
        object["spec"]["timezone"] = json!(timezone);
        object["metadata"]["annotations"]["insight.cyberfabric.com/schedule-origin"] =
            json!(if crons.as_array().ok_or("crons")?.is_empty() {
                "descriptor"
            } else {
                "api"
            });
        if !inherited {
            object["metadata"]["annotations"]
                .as_object_mut()
                .ok_or("annotations")?
                .remove("insight.cyberfabric.com/deployment-schedule");
        }
        let row = stored(crons.clone());
        let object = serde_json::from_value(object)?;
        let (result, live) = request(Some(&row), Some(&object))?;
        assert_eq!(
            result.last_observation.ok_or("status")?.status,
            expected,
            "{crons} {live_cron} {timezone} {inherited}"
        );
        assert_eq!(live.ok_or("live")?.crons, [live_cron]);
    }
    let (missing, _) = request(Some(&stored(json!(["0 6 * * *"]))), None)?;
    assert_eq!(
        missing.last_observation.ok_or("status")?.status,
        ScheduleStatus::Pending
    );
    Ok(())
}

#[tokio::test]
async fn unreadable_cluster_or_database_is_reported_without_a_guessed_status() -> R {
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([Vec::<std::collections::BTreeMap<String, sea_orm::Value>>::new()])
        .into_connection();
    assert!(matches!(
        enrich(&db, &Cluster::disabled(), &mut response()).await,
        Err(Error::Cluster(_))
    ));
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_errors([DbErr::Custom("synthetic failure".to_owned())])
        .into_connection();
    assert!(matches!(
        enrich(&db, &Cluster::disabled(), &mut response()).await,
        Err(Error::Database(_))
    ));
    Ok(())
}

#[test]
fn recorded_schedules_preserve_api_origin() -> R {
    let schedule =
        AppliedSchedule::from_record(vec!["0 6 * * *".to_owned()], "UTC".to_owned(), "api", false)
            .ok_or("schedule missing")?;
    assert_eq!(serde_json::to_value(schedule)?["origin"], "api");
    Ok(())
}

#[tokio::test]
async fn invalid_instances_only_hide_their_own_schedule_status() -> R {
    for invalid in [
        "bad_source",
        "a-source-name-that-makes-the-job-name-exceed-its-limit",
        "identity",
    ] {
        let stub = Stub::start(Scenario::Accept, "example-tracker", "main").await?;
        if invalid == "identity" {
            let mut object = stub.object().await;
            object["metadata"]["labels"]["insight.cyberfabric.com/tenant"] = json!("different");
            stub.replace(object, Scenario::Accept).await;
        }
        let mut rows = response();
        if invalid != "identity" {
            rows.connectors[0].source_id = Some(invalid.to_owned());
        }
        let db = MockDatabase::new(DatabaseBackend::MySql)
            .append_query_results([
                Vec::<std::collections::BTreeMap<String, sea_orm::Value>>::new(),
            ])
            .into_connection();
        enrich(&db, &stub.cluster, &mut rows).await?;
        let bad = rows.connectors[0]
            .schedule_request
            .as_ref()
            .ok_or("invalid row")?;
        assert!(bad.last_observation.is_none(), "{invalid}");
        let reason = bad.read_error.as_deref().ok_or("reason")?;
        let expected = match invalid {
            "bad_source" => "source_id",
            "identity" => "identity",
            _ => "52",
        };
        assert!(reason.contains(expected), "{invalid}: {reason}");
        let good = rows.connectors[1]
            .schedule_request
            .as_ref()
            .ok_or("valid row")?;
        assert!(good.read_error.is_none(), "{invalid}");
        assert_eq!(
            good.last_observation.as_ref().ok_or("status")?.status,
            ScheduleStatus::Pending,
            "{invalid}"
        );
    }
    Ok(())
}
