use super::*;
use dto::{InheritedSchedule, LiveSchedule, PendingReason, ScheduleOrigin, ScheduleStatus};
use kube::api::DynamicObject;
use serde_json::json;

type R = Result<(), Box<dyn std::error::Error>>;

fn live(origin: ScheduleOrigin, cron: &str) -> LiveSchedule {
    LiveSchedule {
        crons: vec![cron.to_owned()],
        timezone: Some("UTC".to_owned()),
        origin: Some(origin),
        suspended: false,
        observed_at: chrono::DateTime::UNIX_EPOCH,
    }
}

#[test]
fn status_requires_live_evidence_before_reporting_applied_or_inherited() {
    let requested = ["0 6 * * *".to_owned()];
    let inherited = InheritedSchedule {
        crons: vec!["0 4 * * *".to_owned()],
        origin: ScheduleOrigin::Descriptor,
    };
    let applied = live(ScheduleOrigin::Api, "0 6 * * *");
    let deployment = live(ScheduleOrigin::Descriptor, "0 4 * * *");
    let mut wrong_timezone = live(ScheduleOrigin::Api, "0 6 * * *");
    wrong_timezone.timezone = None;
    let cases = [
        (
            &requested[..],
            Some(&applied),
            Some(&inherited),
            (ScheduleStatus::Applied, None),
        ),
        (
            &requested[..],
            Some(&applied),
            Some(&inherited),
            (ScheduleStatus::Applied, None),
        ),
        (
            &[][..],
            Some(&deployment),
            Some(&inherited),
            (ScheduleStatus::Inherited, None),
        ),
        (
            &requested[..],
            Some(&deployment),
            Some(&inherited),
            (
                ScheduleStatus::Pending,
                Some(PendingReason::DifferentSchedule),
            ),
        ),
        (
            &requested[..],
            Some(&deployment),
            None,
            (
                ScheduleStatus::Pending,
                Some(PendingReason::DifferentSchedule),
            ),
        ),
        (
            &[][..],
            Some(&applied),
            None,
            (
                ScheduleStatus::Pending,
                Some(PendingReason::AwaitingReconcile),
            ),
        ),
        (
            &requested[..],
            Some(&wrong_timezone),
            Some(&inherited),
            (
                ScheduleStatus::Pending,
                Some(PendingReason::DifferentSchedule),
            ),
        ),
        (
            &[][..],
            Some(&applied),
            Some(&inherited),
            (
                ScheduleStatus::Pending,
                Some(PendingReason::DifferentSchedule),
            ),
        ),
    ];
    for (requested, live, inherited, expected) in cases {
        assert_eq!(
            dto::status(requested, live, inherited),
            expected,
            "{requested:?} {live:?}"
        );
    }
}

#[test]
fn workflow_names_follow_the_shell_instance_label_and_tenant_prefix_rules() -> R {
    for (connector, source, tenant, expected) in [
        (
            "jira",
            "jira-main",
            "00000000-1111",
            "jira-main-00000000-sync",
        ),
        ("jira", "main", "abcdef-.123", "jira-main-abcdef-sync"),
        ("mock", "mock-a", "00000000-0000", "mock-a-00000000-sync"),
        ("jira", "jira", "tenant01", "jira-jira-tenant01-sync"),
        ("jira", "jira-cloud", "tenant01", "jira-cloud-tenant01-sync"),
    ] {
        assert_eq!(
            Instance::parse(connector, source)?.workflow_name(tenant)?,
            expected
        );
    }
    assert!(
        Instance::parse(&"a".repeat(50), "main")?
            .workflow_name("tenant01")
            .is_err()
    );
    Ok(())
}

pub(super) fn object() -> Result<DynamicObject, serde_json::Error> {
    serde_json::from_value(json!({
        "apiVersion": "argoproj.io/v1alpha1", "kind": "CronWorkflow",
        "metadata": {"name": "jira-main-tenant01-sync", "namespace": "example", "uid": "example-uid", "resourceVersion": "7",
            "labels": {"insight.cyberfabric.com/connector": "jira", "insight.cyberfabric.com/tenant": "tenant01", "app.kubernetes.io/component": "connector-sync"},
            "annotations": {workflow::ORIGIN: "descriptor", workflow::DEPLOYMENT: "{\"origin\":\"descriptor\",\"crons\":[\"0 4 * * *\"]}", "example.test/unrelated": "retained"}},
        "spec": {"schedules": ["0 4 * * *"], "timezone": "UTC", "suspend": true,
            "workflowSpec": {"arguments": {"parameters": [{"name": "insight_source_id", "value": "jira-main"}]}}}
    }))
}

#[test]
fn identity_requires_all_labels_namespace_name_and_exactly_one_source_parameter() -> R {
    let instance = Instance::parse("jira", "jira-main")?;
    let object = object()?;
    assert!(workflow::identity_matches(
        &object, &instance, "example", "tenant01"
    ));
    for pointer in [
        "/metadata/name",
        "/metadata/namespace",
        "/metadata/labels/insight.cyberfabric.com~1connector",
        "/metadata/labels/insight.cyberfabric.com~1tenant",
        "/metadata/labels/app.kubernetes.io~1component",
        "/spec/workflowSpec/arguments/parameters/0/value",
    ] {
        let mut value = serde_json::to_value(&object)?;
        *value.pointer_mut(pointer).ok_or("missing fixture field")? = json!("wrong");
        let changed = serde_json::from_value(value)?;
        assert!(
            !workflow::identity_matches(&changed, &instance, "example", "tenant01"),
            "{pointer}"
        );
    }
    let mut duplicate = object;
    duplicate.data["spec"]["workflowSpec"]["arguments"]["parameters"]
        .as_array_mut()
        .ok_or("parameters")?
        .push(json!({"name":"insight_source_id", "value":"jira-main"}));
    assert!(!workflow::identity_matches(
        &duplicate, &instance, "example", "tenant01"
    ));
    Ok(())
}

#[test]
fn merge_patch_fences_identity_and_version_and_owns_only_the_schedule_fields() -> R {
    let object = object()?;
    let target = InheritedSchedule {
        crons: vec!["0 6 * * *".to_owned()],
        origin: ScheduleOrigin::Api,
    };
    assert_eq!(
        workflow::patch(&object, &target)?,
        json!({
            "metadata": {"uid":"example-uid", "resourceVersion":"7", "annotations": {workflow::ORIGIN:"api"}},
            "spec": {"schedules":["0 6 * * *"], "timezone":"UTC"}
        })
    );
    let mut unfenced = object;
    unfenced.metadata.uid = None;
    assert!(workflow::patch(&unfenced, &target).is_err());
    Ok(())
}

#[test]
fn tombstones_need_deployment_but_puts_do_not() -> R {
    let object = object()?;
    let target = workflow::desired(&[], workflow::inherited(&object)).ok_or("missing restore")?;
    assert_eq!(target.crons, ["0 4 * * *"]);
    assert_eq!(target.origin, ScheduleOrigin::Descriptor);
    assert!(workflow::desired(&["0 6 * * *".to_owned()], None).is_some());
    assert!(workflow::desired(&[], None).is_none());
    let mut object = object;
    for annotation in [
        "{}",
        "{\"origin\":\"api\",\"crons\":[\"0 6 * * *\"]}",
        "{\"origin\":\"default\",\"crons\":[]}",
    ] {
        object
            .metadata
            .annotations
            .as_mut()
            .ok_or("annotations")?
            .insert(workflow::DEPLOYMENT.to_owned(), annotation.to_owned());
        assert!(workflow::inherited(&object).is_none(), "{annotation}");
    }
    Ok(())
}

#[test]
fn drift_is_a_difference_in_crons_timezone_or_origin_and_keeps_suspend() -> R {
    let object = object()?;
    let requested = ["0 6 * * *".to_owned()];
    let target = workflow::desired(&requested, workflow::inherited(&object)).ok_or("target")?;
    assert!(!workflow::matches(&workflow::live(&object)?, &target));
    let mut observed = live(ScheduleOrigin::Api, "0 6 * * *");
    assert!(workflow::matches(&observed, &target));
    observed.timezone = None;
    assert!(!workflow::matches(&observed, &target));
    observed.timezone = Some("UTC".to_owned());
    observed.origin = Some(ScheduleOrigin::Default);
    assert!(!workflow::matches(&observed, &target));
    Ok(())
}

#[test]
fn if_match_requires_one_quoted_unsigned_decimal_revision() -> R {
    for (raw, expected) in [
        ("\"0\"", 0),
        ("\"0003\"", 3),
        ("\"18446744073709551615\"", u64::MAX),
    ] {
        assert_eq!(Revision::parse(Some(raw))?.0, expected, "{raw}");
    }
    for raw in [
        "",
        "0",
        "*",
        "W/\"3\"",
        "\"3\", \"4\"",
        "\"-1\"",
        "\"+1\"",
        "\"\"",
        " \"3\"",
        "\"18446744073709551616\"",
    ] {
        assert!(
            matches!(Revision::parse(Some(raw)), Err(Error::InvalidMatch)),
            "{raw}"
        );
    }
    assert!(matches!(Revision::parse(None), Err(Error::MissingMatch)));
    Ok(())
}

mod reset;

#[test]
fn missing_jobs_are_not_scheduled() {
    for requested in [vec!["0 6 * * *".to_owned()], Vec::new()] {
        assert_eq!(
            dto::status(&requested, None, None),
            (ScheduleStatus::Pending, Some(PendingReason::NoCronworkflow)),
            "{requested:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn schedule_slot_wait_is_bounded() {
    let slots = tokio::sync::Semaphore::new(0);
    let started = tokio::time::Instant::now();
    let result = tokio::time::timeout(std::time::Duration::from_secs(6), slot(&slots)).await;
    assert!(matches!(result, Ok(Err(Error::LockBusy))));
    assert_eq!(started.elapsed(), std::time::Duration::from_secs(5));
}

#[test]
fn paused_jobs_take_precedence_over_every_saved_schedule() {
    for requested in [vec!["0 6 * * *".to_owned()], Vec::new()] {
        for origin in [ScheduleOrigin::Api, ScheduleOrigin::Descriptor] {
            let mut observed = live(origin, "0 6 * * *");
            observed.suspended = true;
            assert_eq!(
                dto::status(&requested, Some(&observed), None),
                (ScheduleStatus::Paused, None),
                "{requested:?} {origin:?}"
            );
        }
    }
}

#[test]
fn release_failure_is_logged() {
    let output =
        insight_log_context::test_support::capture_output(|| release(Err(Error::LockBusy)));
    assert!(output.contains("schedule lock release failed"));
}
