use std::time::Duration;

use super::*;
use crate::domain::connector_schedules::{Instance, lock, save_hooks};
use crate::infra::cluster_stub::Scenario;

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn dropped_save_and_reset_requests_finish_each_locked_step_and_release_the_lock() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    for method in ["PUT", "DELETE"] {
        for step in [
            "slot", "lock", "revision", "write", "patch", "readback", "apply", "release",
        ] {
            cancelled(&db, method, step, false).await?;
        }
        for step in ["confirm", "rollback", "apply", "release"] {
            cancelled(&db, method, step, true).await?;
        }
    }
    Ok(())
}

async fn cancelled(db: &DatabaseConnection, method: &str, step: &'static str, refused: bool) -> R {
    let (connector, uri) = identity();
    let previous_cron = if method == "DELETE" {
        "0 6 * * *"
    } else {
        "0 4 * * *"
    };
    let before = revisions::arrange_schedule(db, &connector, 7, json!([previous_cron])).await?;
    let history = Mock::new();
    history.add(handlers::provide([configured_instance_record()]));
    let h = harness_for_cluster(
        db.clone(),
        history.url(),
        context("user", &[])?,
        true,
        Scenario::Accept,
        &connector,
    )
    .await?;
    let mut object = h.workflow.object().await;
    object["spec"]["schedules"] = json!([previous_cron]);
    object["metadata"]["annotations"]["insight.cyberfabric.com/schedule-origin"] = json!("api");
    h.workflow
        .replace(
            object,
            if refused {
                Scenario::Refuse(403)
            } else {
                Scenario::Accept
            },
        )
        .await;
    let gate = save_hooks::install(&connector, step);
    let request = request(method, &uri, Some("\"7\""), &json!({"crons":["0 8 * * *"]}))?;
    let handler = tokio::spawn(h.app.clone().oneshot(request));
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified()).await?;
    handler.abort();
    let _ = handler.await;
    gate.resume.notify_one();
    tokio::time::timeout(Duration::from_secs(3), gate.finished.notified()).await?;
    let instance = Instance::parse(&connector, "main")?;
    lock::acquire(db, &instance).await?.release().await?;
    let row = Entity::find_by_id((connector.clone(), "main".to_owned()))
        .one(db)
        .await?
        .ok_or("saved row")?;
    let expected = if refused {
        previous_cron
    } else if method == "DELETE" {
        "0 4 * * *"
    } else {
        "0 8 * * *"
    };
    assert_eq!(
        h.workflow.object().await["spec"]["schedules"],
        json!([expected]),
        "{method} {step}"
    );
    if refused {
        assert_eq!(row, before, "{method} {step}");
    } else {
        assert_eq!(row.revision, 8, "{method} {step}");
        assert_eq!(
            row.crons,
            if method == "DELETE" {
                json!([])
            } else {
                json!([expected])
            },
            "{method} {step}"
        );
    }
    save_hooks::remove(&connector);
    clear(db, &connector).await
}
