use super::*;
use crate::domain::connector_schedules::dto::PendingReason;
use crate::infra::cluster_stub::{Scenario, Stub};
use serde_json::json;

type R = Result<(), Box<dyn std::error::Error>>;

fn response(status: ScheduleStatus, reason: Option<PendingReason>) -> ScheduleResponse {
    ScheduleResponse {
        connector: "example-tracker".to_owned(),
        source_id: "main".to_owned(),
        revision: 1,
        requested: None,
        live: None,
        inherited: None,
        status: Some(status),
        read_error: None,
        pending_reason: reason,
        next_reconcile_at: None,
        reconcile_interval_secs: None,
    }
}

#[test]
fn reconcile_fires_follow_all_crons_timezone_and_calendar_rules() -> R {
    let now = "2026-01-15T12:00:00Z".parse()?;
    for (spec, next, interval) in [
        (
            json!({"schedules":["*/15 * * * *"], "timezone":"UTC"}),
            "2026-01-15T12:15:00Z",
            900,
        ),
        (
            json!({"schedule":"0 9 * * *", "timezone":"Asia/Kolkata"}),
            "2026-01-16T03:30:00Z",
            86400,
        ),
        (
            json!({"schedules":["0 * * * *", "10 * * * *"], "timezone":"UTC"}),
            "2026-01-15T12:10:00Z",
            3000,
        ),
        (
            json!({"schedule":"0 0 1 * MON", "timezone":"UTC"}),
            "2026-01-19T00:00:00Z",
            604_800,
        ),
    ] {
        let object: DynamicObject = serde_json::from_value(json!({"metadata":{},"spec":spec}))?;
        let actual = reconcile_times(&object, now).ok_or("next fires")?;
        assert_eq!(actual, (next.parse()?, interval), "spec={spec}");
    }
    for (now, next, interval) in [
        ("2026-03-08T06:00:00Z", "2026-03-09T06:30:00Z", 86400),
        ("2026-11-01T05:00:00Z", "2026-11-01T05:30:00Z", 3600),
    ] {
        let hour = if now.contains("03-08") { 2 } else { 1 };
        let object = serde_json::from_value(
            json!({"metadata":{},"spec":{"schedule":format!("30 {hour} * * *"),"timezone":"America/New_York"}}),
        )?;
        assert_eq!(
            reconcile_times(&object, now.parse()?),
            Some((next.parse()?, interval)),
            "now={now}"
        );
    }
    Ok(())
}

#[test]
fn suspended_invalid_or_unreadable_reconcile_shapes_have_no_estimate() -> R {
    let now = "2026-01-15T12:00:00Z".parse()?;
    for spec in [
        json!({}),
        json!({"schedule":"not a cron"}),
        json!({"schedule":"*/15 * * * *", "suspend":true}),
        json!({"schedule":"*/15 * * * *", "timezone":"Invalid/Zone"}),
        json!({"schedules":["*/15 * * * *", "bad"]}),
    ] {
        let object = serde_json::from_value(json!({"metadata":{},"spec":spec}))?;
        assert_eq!(reconcile_times(&object, now), None, "spec={spec}");
    }
    Ok(())
}

#[tokio::test]
async fn timing_fields_are_only_present_for_their_waiting_status() -> R {
    let stub = Stub::start(Scenario::Accept, "example-tracker", "main").await?;
    stub.set_reconcile(
        json!({"metadata":{}, "spec":{"schedules":["*/15 * * * *"],"timezone":"UTC"}}),
    )
    .await;
    let cluster = stub
        .cluster
        .clone()
        .with_reconcile_workflow("example-reconcile-loop".to_owned());
    for (status, reason, reconcile) in [
        (
            ScheduleStatus::Pending,
            Some(PendingReason::NoCronworkflow),
            true,
        ),
        (
            ScheduleStatus::Pending,
            Some(PendingReason::AwaitingReconcile),
            true,
        ),
        (
            ScheduleStatus::Pending,
            Some(PendingReason::DifferentSchedule),
            true,
        ),
        (ScheduleStatus::Applied, None, false),
        (ScheduleStatus::Inherited, None, false),
    ] {
        let mut response = response(status, reason);
        enrich(&mut response, &cluster).await;
        assert_eq!(
            response.next_reconcile_at.is_some(),
            reconcile,
            "status={:?} reason={reason:?}",
            response.status
        );
        assert_eq!(response.reconcile_interval_secs, reconcile.then_some(900));
    }
    Ok(())
}

#[tokio::test]
async fn missing_suspended_refused_or_invalid_reconcile_reads_never_fail_the_schedule() -> R {
    for (scenario, spec) in [
        (Scenario::Accept, None),
        (
            Scenario::Accept,
            Some(json!({"schedule":"*/15 * * * *", "suspend":true})),
        ),
        (Scenario::Accept, Some(json!({"schedule":"bad"}))),
        (
            Scenario::Refuse(403),
            Some(json!({"schedule":"*/15 * * * *"})),
        ),
    ] {
        let stub = Stub::start(scenario, "example-tracker", "main").await?;
        if let Some(spec) = spec {
            stub.set_reconcile(json!({"metadata":{}, "spec":spec}))
                .await;
        }
        let cluster = stub
            .cluster
            .clone()
            .with_reconcile_workflow("example-reconcile-loop".to_owned());
        let mut response = response(
            ScheduleStatus::Pending,
            Some(PendingReason::AwaitingReconcile),
        );
        enrich(&mut response, &cluster).await;
        assert!(response.next_reconcile_at.is_none());
        assert!(response.reconcile_interval_secs.is_none());
    }
    Ok(())
}
