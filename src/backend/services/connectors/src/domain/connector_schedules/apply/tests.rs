use super::*;
use crate::infra::cluster_stub::{Scenario, Stub};

type R = Result<(), Box<dyn std::error::Error>>;

mod logging;

#[test]
fn change_logs_name_the_instance_revision_and_changed_crons() -> R {
    let instance = Instance::parse("example-tracker", "main")?;
    let live = |cron: &str| LiveSchedule {
        crons: vec![cron.to_owned()],
        timezone: Some("UTC".to_owned()),
        origin: None,
        suspended: false,
        observed_at: chrono::DateTime::UNIX_EPOCH,
    };
    let before = live("0 4 * * *");
    let after = InheritedSchedule {
        crons: vec!["0 6 * * *".to_owned()],
        origin: super::super::dto::ScheduleOrigin::Api,
    };
    let output = insight_log_context::test_support::capture_output(|| {
        log_change(7, &instance, Some(&before), &after);
    });
    let lines = output
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["level"], "INFO");
    let fields = &lines[0]["fields"];
    assert_eq!(fields["message"], "sync job schedule changed");
    assert_eq!(fields["connector"], "example-tracker");
    assert_eq!(fields["source_id"], "main");
    assert_eq!(fields["revision"], 7);
    assert_eq!(fields["from_crons"], "[\"0 4 * * *\"]");
    assert_eq!(fields["to_crons"], "[\"0 6 * * *\"]");
    Ok(())
}

#[test]
fn save_logs_each_accepted_patch_and_skips_unchanged_schedules() -> R {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    for (scenario, expected) in [
        (Scenario::Accept, 1),
        (Scenario::Conflict, 1),
        (Scenario::Refuse(403), 0),
        (Scenario::Mismatch, 1),
        (Scenario::Unreadable, 1),
        (Scenario::Missing, 0),
        (Scenario::AnnotationAbsent, 1),
    ] {
        let mut result: R = Ok(());
        let output = insight_log_context::test_support::capture_output(|| {
            result = runtime.block_on(async {
                let stub = Stub::start(scenario, "example-tracker", "main").await?;
                let instance = Instance::parse("example-tracker", "main")?;
                let (_, failure) =
                    attempt(&stub.cluster, &instance, &["0 6 * * *".to_owned()], 7).await;
                if failure.is_none() {
                    attempt(&stub.cluster, &instance, &["0 6 * * *".to_owned()], 7).await;
                }
                Ok::<(), Box<dyn std::error::Error>>(())
            });
        });
        result?;
        assert_eq!(
            output.matches("sync job schedule changed").count(),
            expected
        );
    }
    Ok(())
}

#[tokio::test]
async fn apply_reads_back_and_retries_a_version_conflict_once() -> R {
    for conflict in [false, true] {
        let stub = Stub::start(
            if conflict {
                Scenario::Conflict
            } else {
                Scenario::Accept
            },
            "jira",
            "jira-main",
        )
        .await?;
        let mut object = stub.object().await;
        object["spec"]["suspend"] = serde_json::json!(true);
        stub.replace(
            object,
            if conflict {
                Scenario::Conflict
            } else {
                Scenario::Accept
            },
        )
        .await;
        let instance = Instance::parse("jira", "jira-main")?;
        let (observation, error) =
            attempt(&stub.cluster, &instance, &["0 6 * * *".to_owned()], 7).await;
        assert!(error.is_none());
        assert_eq!(observation.live.ok_or("live")?.crons, ["0 6 * * *"]);
        let patches = stub.patches().await;
        assert_eq!(
            stub.field_managers().await,
            vec![Some("insight-connectors".to_owned()); patches.len()]
        );
        assert_eq!(patches.len(), if conflict { 2 } else { 1 });
        if conflict {
            assert_eq!(patches[1]["metadata"]["resourceVersion"], "8");
        }
        let object = stub.object().await;
        assert_eq!(object["spec"]["suspend"], true);
        assert_eq!(
            object["metadata"]["annotations"]["example.test/unrelated"],
            "retained"
        );
    }
    Ok(())
}

#[tokio::test]
async fn refused_patches_keep_the_cluster_message_and_unverified_reads_are_explicit() -> R {
    for (scenario, expected) in [
        (Scenario::Refuse(403), "the cluster refused the change:"),
        (Scenario::Refuse(422), "the cluster refused the change:"),
        (Scenario::Refuse(500), "the cluster returned an error:"),
        (Scenario::AlwaysConflict, "the cluster refused the change:"),
        (Scenario::Timeout, "the cluster did not answer in time"),
        (Scenario::Mismatch, MISMATCH),
    ] {
        let refused = matches!(scenario, Scenario::Refuse(_));
        let stub = Stub::start(scenario, "jira", "jira-main").await?;
        let instance = Instance::parse("jira", "jira-main")?;
        let (_, error) = attempt(&stub.cluster, &instance, &["0 6 * * *".to_owned()], 7).await;
        let message = error.ok_or("missing failure")?;
        assert!(message.starts_with(expected));
        if refused {
            assert!(message.contains("synthetic private server detail"));
        }
    }
    Ok(())
}

#[tokio::test]
async fn missing_jobs_keep_requests_and_only_resets_require_the_annotation() -> R {
    for scenario in [Scenario::Missing, Scenario::AnnotationAbsent] {
        let stub = Stub::start(scenario, "jira", "jira-main").await?;
        let instance = Instance::parse("jira", "jira-main")?;
        let (_, error) = attempt(&stub.cluster, &instance, &[], 7).await;
        assert_eq!(
            error.is_some(),
            stub.cluster
                .get(&instance.workflow_name("tenant01")?)
                .await?
                .is_some()
        );
        assert!(stub.patches().await.is_empty());
        let (observed, error) =
            attempt(&stub.cluster, &instance, &["0 6 * * *".to_owned()], 7).await;
        assert!(error.is_none());
        if observed.live.is_some() {
            assert_eq!(stub.patches().await.len(), 1);
        }
    }
    Ok(())
}

#[tokio::test]
async fn saving_restores_the_request_and_clearing_restores_deployment() -> R {
    let stub = Stub::start(Scenario::default(), "jira", "jira-main").await?;
    let instance = Instance::parse("jira", "jira-main")?;
    let crons = vec!["0 6 * * *".to_owned()];
    assert!(
        attempt(&stub.cluster, &instance, &crons, 7)
            .await
            .1
            .is_none()
    );
    stub.revert().await;
    assert!(
        attempt(&stub.cluster, &instance, &crons, 7)
            .await
            .1
            .is_none()
    );
    let (restored, error) = attempt(&stub.cluster, &instance, &[], 7).await;
    assert!(error.is_none());
    assert_eq!(
        restored.live.ok_or("live")?.origin,
        Some(super::super::dto::ScheduleOrigin::Descriptor)
    );
    assert_eq!(stub.patches().await.len(), 3);
    Ok(())
}

#[tokio::test]
async fn refused_or_mismatched_resets_never_report_inherited_from_origin_alone() -> R {
    use sea_orm::{DbBackend, MockDatabase, MockExecResult};

    use crate::infra::db::entities::connector_schedules::Model;

    for scenario in [Scenario::Refuse(403), Scenario::Mismatch] {
        let stub = Stub::start(Scenario::Accept, "example-tracker", "main").await?;
        let mut object = stub.object().await;
        object["spec"]["schedules"] = serde_json::json!(["0 6 * * *"]);
        stub.replace(object, scenario).await;
        let instance = Instance::parse("example-tracker", "main")?;
        let now = chrono::Utc::now();
        let db = MockDatabase::new(DbBackend::MySql)
            .append_query_results([vec![Model {
                connector: "example-tracker".to_owned(),
                source_id: "main".to_owned(),
                crons: serde_json::json!([]),
                revision: 2,
                updated_by: uuid::Uuid::from_u128(1),
                updated_at: now,
            }]])
            .append_exec_results([MockExecResult::default()])
            .into_connection();
        assert!(matches!(
            stored(&db, &stub.cluster, &instance).await,
            Err(Error::NotChanged(_))
        ));
        assert_eq!(
            stub.object().await["spec"]["schedules"],
            serde_json::json!(["0 6 * * *"])
        );
    }
    Ok(())
}

#[test]
fn a_lost_patch_response_is_confirmed_and_logged_once_from_the_previous_schedule() -> R {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut result: R = Ok(());
    let output = insight_log_context::test_support::capture_output(|| {
        result = runtime.block_on(async {
            let stub = Stub::start(Scenario::LostResponse, "example-tracker", "main").await?;
            let instance = Instance::parse("example-tracker", "main")?;
            let (observed, failure) =
                attempt(&stub.cluster, &instance, &["0 6 * * *".to_owned()], 7).await;
            assert!(failure.is_none(), "{failure:?}");
            assert_eq!(
                observed.live.ok_or("confirmation missing")?.crons,
                ["0 6 * * *"]
            );
            assert_eq!(stub.patches().await.len(), 1);
            Ok(())
        });
    });
    result?;

    let lines = output
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()?;
    let changes: Vec<_> = lines
        .iter()
        .filter(|line| line["fields"]["message"] == "sync job schedule changed")
        .collect();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["level"], "INFO");
    let fields = &changes[0]["fields"];
    assert_eq!(fields["connector"], "example-tracker");
    assert_eq!(fields["source_id"], "main");
    assert_eq!(fields["revision"], 7);
    assert_eq!(fields["from_crons"], "[\"0 4 * * *\"]");
    assert_eq!(fields["to_crons"], "[\"0 6 * * *\"]");
    Ok(())
}

#[tokio::test]
async fn accepted_or_unknown_patches_keep_the_request_when_confirmation_is_unavailable() -> R {
    for scenario in [
        Scenario::Unreadable,
        Scenario::LostUnreadable,
        Scenario::MalformedReadback,
    ] {
        let stub = Stub::start(scenario, "example-tracker", "main").await?;
        let instance = Instance::parse("example-tracker", "main")?;
        let (observed, failure) =
            attempt(&stub.cluster, &instance, &["0 6 * * *".to_owned()], 7).await;
        assert!(failure.is_none(), "{failure:?}");
        assert!(observed.read_error.is_some());
        assert!(observed.live.is_none());
        assert_eq!(
            stub.object().await["spec"]["schedules"],
            serde_json::json!(["0 6 * * *"])
        );
    }
    Ok(())
}

#[tokio::test]
async fn unknown_patches_are_fenced_before_rollback_or_confirmed_after_a_conflict() -> R {
    for late_wins in [false, true] {
        let stub = Stub::start(Scenario::LatePatch(late_wins), "example-tracker", "main").await?;
        let instance = Instance::parse("example-tracker", "main")?;
        let (observed, failure) =
            attempt(&stub.cluster, &instance, &["0 6 * * *".to_owned()], 7).await;
        stub.finish_pending_patch().await;
        assert_eq!(failure.is_some(), !late_wins, "late wins: {late_wins}");
        let expected = if late_wins { "0 6 * * *" } else { "0 4 * * *" };
        assert_eq!(observed.live.ok_or("live")?.crons, [expected]);
        assert_eq!(
            stub.object().await["spec"]["schedules"],
            serde_json::json!([expected])
        );
        let patches = stub.patches().await;
        assert_eq!(patches.len(), 2);
        assert_eq!(patches[1]["metadata"]["resourceVersion"], "7");
        assert!(patches[1].get("spec").is_none());
    }
    Ok(())
}
