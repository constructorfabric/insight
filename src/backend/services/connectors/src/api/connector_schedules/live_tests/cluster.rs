use sea_orm::{ConnectionTrait, DbBackend, Statement};

use super::*;
use crate::infra::cluster_stub::Scenario;

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn failed_saves_restore_the_exact_previous_row_and_lost_responses_are_confirmed() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    for existing in [false, true] {
        for (scenario, expected, status) in [
            (Scenario::Accept, StatusCode::OK, "applied"),
            (Scenario::Conflict, StatusCode::OK, "applied"),
            (Scenario::LostResponse, StatusCode::OK, "applied"),
            (Scenario::LatePatch(true), StatusCode::OK, "applied"),
            (
                Scenario::LatePatch(false),
                StatusCode::SERVICE_UNAVAILABLE,
                "",
            ),
            (Scenario::AnnotationAbsent, StatusCode::OK, "applied"),
            (Scenario::Missing, StatusCode::OK, "pending"),
            (Scenario::Refuse(403), StatusCode::SERVICE_UNAVAILABLE, ""),
            (Scenario::Refuse(422), StatusCode::SERVICE_UNAVAILABLE, ""),
            (Scenario::Refuse(500), StatusCode::SERVICE_UNAVAILABLE, ""),
            (
                Scenario::AlwaysConflict,
                StatusCode::SERVICE_UNAVAILABLE,
                "",
            ),
            (Scenario::Timeout, StatusCode::SERVICE_UNAVAILABLE, ""),
            (Scenario::Mismatch, StatusCode::SERVICE_UNAVAILABLE, ""),
            (Scenario::Unreadable, StatusCode::OK, ""),
            (Scenario::LostUnreadable, StatusCode::OK, ""),
        ] {
            let (connector, uri) = identity();
            let before = if existing {
                Some(revisions::arrange_schedule(&db, &connector, 7, json!(["0 2 * * *"])).await?)
            } else {
                None
            };
            let revision = if existing { "\"7\"" } else { "\"0\"" };
            let history = Mock::new();
            let h = harness_for_cluster(
                db.clone(),
                history.url(),
                context("user", &[])?,
                true,
                scenario,
                &connector,
            )
            .await?;
            let mut object = h.workflow.object().await;
            let previous_crons = before
                .as_ref()
                .map_or_else(|| json!(["0 4 * * *"]), |row| row.crons.clone());
            object["spec"]["schedules"] = previous_crons.clone();
            let before_job = object.clone();
            h.workflow.set_object(object).await;
            let (code, response) =
                send((&h, &history), "PUT", &uri, Some(revision), &["0 6 * * *"]).await?;
            h.workflow.finish_pending_patch().await;
            assert_eq!(code, expected, "{response}");
            let row = Entity::find_by_id((connector.clone(), "main".to_owned()))
                .one(&db)
                .await?;
            if expected == StatusCode::OK {
                if status.is_empty() {
                    assert!(response["status"].is_null());
                    assert!(response["read_error"].is_string());
                } else {
                    assert_eq!(response["status"], status);
                }
                if status != "pending" {
                    assert_eq!(
                        h.workflow.object().await["spec"]["schedules"],
                        json!(["0 6 * * *"])
                    );
                }
                assert_eq!(response["revision"], if existing { 8 } else { 1 });
                assert_eq!(row.ok_or("saved row missing")?.crons, json!(["0 6 * * *"]));
                assert!(
                    !response
                        .as_object()
                        .ok_or("response")?
                        .contains_key("error")
                );
            } else {
                assert_eq!(row, before, "rollback must preserve every field");
                let after_job = h.workflow.object().await;
                assert_eq!(after_job["spec"], before_job["spec"]);
                assert_eq!(
                    schedule_annotations(&after_job),
                    before_job["metadata"]["annotations"]
                );
                assert_not_changed(&response)?;
            }
            clear(&db, &connector).await?;
        }
    }
    Ok(())
}

fn assert_not_changed(response: &Value) -> R {
    let detail = response["detail"].as_str().ok_or("detail")?;
    assert!(detail.contains("schedule was not changed"), "{detail}");
    assert!(
        detail.contains("synthetic private server detail")
            || detail.contains("timed out")
            || detail.contains("different schedule"),
        "{detail}"
    );
    Ok(())
}

fn schedule_annotations(job: &Value) -> Value {
    let mut annotations = job["metadata"]["annotations"].clone();
    if let Some(annotations) = annotations.as_object_mut() {
        annotations.remove("insight.cyberfabric.com/schedule-fence");
    }
    annotations
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn save_uses_three_second_cluster_calls_and_keeps_unconfirmed_requests() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    for scenario in [Scenario::Timeout, Scenario::LostUnreadable] {
        let unknown = scenario == Scenario::LostUnreadable;
        let (connector, uri) = identity();
        let history = Mock::new();
        let h = harness_for_cluster(
            db.clone(),
            history.url(),
            context("user", &[])?,
            true,
            scenario,
            &connector,
        )
        .await?;
        let started = tokio::time::Instant::now();
        let (code, answer) = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            send((&h, &history), "PUT", &uri, Some("\"0\""), &["0 6 * * *"]),
        )
        .await??;
        assert!(started.elapsed() >= std::time::Duration::from_secs(3));
        let row = Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?;
        if unknown {
            assert_eq!(code, StatusCode::OK, "{answer}");
            assert!(answer["status"].is_null());
            assert!(answer["read_error"].is_string());
            assert_eq!(row.ok_or("saved request")?.crons, json!(["0 6 * * *"]));
        } else {
            assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
            assert!(
                answer["detail"]
                    .as_str()
                    .ok_or("detail")?
                    .contains("did not answer in time")
            );
            assert!(row.is_none());
        }
        clear(&db, &connector).await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn resetting_without_deployment_changes_nothing_and_same_schedule_saves_still_apply() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = identity();
    let before = revisions::arrange_schedule(&db, &connector, 7, json!(["0 6 * * *"])).await?;
    let history = Mock::new();
    let h = harness_for_cluster(
        db.clone(),
        history.url(),
        context("user", &[])?,
        true,
        Scenario::AnnotationAbsent,
        &connector,
    )
    .await?;
    let (code, refusal) = send((&h, &history), "DELETE", &uri, Some("\"7\""), &[]).await?;
    assert_eq!(code, StatusCode::CONFLICT);
    assert!(
        refusal["detail"]
            .as_str()
            .ok_or("detail")?
            .contains("Reset requires the deployment schedule annotation")
    );
    assert!(h.workflow.patches().await.is_empty());
    assert_eq!(
        h.workflow.object().await["spec"]["schedules"],
        json!(["0 4 * * *"])
    );
    assert_eq!(
        Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?,
        Some(before.clone())
    );
    let (code, response) = send((&h, &history), "PUT", &uri, Some("\"7\""), &["0 6 * * *"]).await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(response["status"], "applied");
    assert_eq!(response["revision"], 7);
    assert_eq!(h.workflow.patches().await.len(), 1);
    assert_eq!(
        Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?,
        Some(before)
    );
    clear(&db, &connector).await
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL and trigger privileges"]
async fn failed_restore_reports_saved_but_not_applied() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = identity();
    revisions::arrange_schedule(&db, &connector, 7, json!(["0 2 * * *"])).await?;
    let trigger = format!("rollback_{}", Uuid::now_v7().simple());
    db.execute_raw(Statement::from_string(DbBackend::MySql, format!(
        "CREATE TRIGGER {trigger} BEFORE UPDATE ON connector_schedules FOR EACH ROW BEGIN IF NEW.connector = '{connector}' AND NEW.revision < OLD.revision THEN SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'synthetic restore failure'; END IF; END"
    ))).await?;
    let history = Mock::new();
    let h = harness_for_cluster(
        db.clone(),
        history.url(),
        context("user", &[])?,
        true,
        Scenario::Refuse(403),
        &connector,
    )
    .await?;
    let result = send((&h, &history), "PUT", &uri, Some("\"7\""), &["0 6 * * *"]).await;
    db.execute_raw(Statement::from_string(
        DbBackend::MySql,
        format!("DROP TRIGGER {trigger}"),
    ))
    .await?;
    let (code, response) = result?;
    assert_eq!(
        h.workflow.object().await["spec"]["schedules"],
        json!(["0 4 * * *"])
    );
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        response["detail"]
            .as_str()
            .ok_or("detail")?
            .contains("request is saved but not applied")
    );
    assert_eq!(
        Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?
            .ok_or("row")?
            .revision,
        8
    );
    clear(&db, &connector).await
}
