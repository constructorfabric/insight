use std::time::Duration;

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tracing::Instrument;

use super::*;
use crate::infra::cluster_stub::Scenario;

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn saves_bound_repository_pool_waits_and_database_statements() -> R {
    let url = std::env::var("INTEGRATION_TESTS_MARIADB_URL")?;
    let db = crate::infra::db::connect(&url).await?;
    Migration.up(&SchemaManager::new(&db)).await?;
    let started = tokio::time::Instant::now();
    let statement = db.query_one_raw(Statement::from_string(DbBackend::MySql, "SELECT SLEEP(10)"));
    assert!(
        tokio::time::timeout(Duration::from_secs(2), statement)
            .await?
            .is_err()
    );
    assert!(started.elapsed() >= Duration::from_secs(1));

    for checkpoint in ["slot", "lock", "revision"] {
        pool_exhaustion_leaves_the_schedule_unchanged(&db, checkpoint).await?;
    }
    removed_reset_pool_exhaustion_leaves_the_schedule_unchanged(&db).await?;
    Ok(())
}

async fn removed_reset_pool_exhaustion_leaves_the_schedule_unchanged(db: &DatabaseConnection) -> R {
    let (connector, uri) = identity();
    let history = Mock::new();
    history.add(handlers::provide(Vec::<ConfiguredInstanceRecord>::new()));
    let h = harness(db.clone(), history.url(), context("user", &[])?, true).await?;
    let mut connections = Vec::new();
    for _ in 0..10 {
        connections.push(db.get_mysql_connection_pool().acquire().await?);
    }
    let (status, answer) = send_request(&h, "DELETE", &uri, Some("\"0\""), &[]).await?;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
    let detail = answer["detail"].as_str().ok_or("missing detail")?;
    assert!(
        detail.starts_with("The schedule was not changed:"),
        "{answer}"
    );
    assert!(detail.contains("try again"), "{answer}");
    assert!(h.workflow.patches().await.is_empty());
    drop(connections);
    assert!(
        Entity::find_by_id((connector, "main".to_owned()))
            .one(db)
            .await?
            .is_none()
    );
    Ok(())
}

async fn pool_exhaustion_leaves_the_schedule_unchanged(
    db: &DatabaseConnection,
    checkpoint: &'static str,
) -> R {
    let (connector, uri) = identity();
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
    let gate = crate::domain::connector_schedules::save_hooks::install(&connector, checkpoint);
    let handler = tokio::spawn(h.app.clone().oneshot(request(
        "PUT",
        &uri,
        Some("\"0\""),
        &json!({"crons":["0 6 * * *"]}),
    )?));
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified()).await?;
    let mut connections = Vec::new();
    for _ in 0..if checkpoint == "slot" { 10 } else { 9 } {
        connections.push(db.get_mysql_connection_pool().acquire().await?);
    }
    let started = tokio::time::Instant::now();
    gate.resume.notify_one();
    let response = tokio::time::timeout(Duration::from_secs(2), handler).await???;
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "{checkpoint}"
    );
    let answer = body(response).await?;
    let detail = answer["detail"].as_str().ok_or("missing detail")?;
    assert!(
        detail.starts_with("The schedule was not changed:"),
        "{checkpoint}: {answer}"
    );
    assert!(detail.contains("try again"), "{checkpoint}: {answer}");
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert_eq!(h.state.schedule_slots.available_permits(), 4);
    assert!(h.workflow.patches().await.is_empty());
    crate::domain::connector_schedules::save_hooks::remove(&connector);
    drop(connections);
    assert!(
        Entity::find_by_id((connector, "main".to_owned()))
            .one(db)
            .await?
            .is_none(),
        "{checkpoint}"
    );
    Ok(())
}

#[test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
fn spawned_save_logs_keep_the_request_span() -> R {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut result: R = Ok(());
    let output = insight_log_context::test_support::capture_output(|| {
        result = runtime.block_on(async {
            let db = connect_or_skip().await?.ok_or("database URL required")?;
            let (connector, uri) = identity();
            let history = Mock::new();
            let h = harness_for_cluster(
                db.clone(),
                history.url(),
                context("user", &[])?,
                true,
                Scenario::Accept,
                &connector,
            )
            .await?;
            let span = tracing::info_span!(
                "log_ctx",
                correlation_id = "synthetic-correlation",
                tenant_id = "synthetic-tenant"
            );
            let (code, answer) = send((&h, &history), "PUT", &uri, Some("\"0\""), &["0 6 * * *"])
                .instrument(span)
                .await?;
            assert_eq!(code, StatusCode::OK, "{answer}");
            clear(&db, &connector).await
        });
    });
    result?;
    let lines = output
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    let line = lines
        .iter()
        .find(|line| line["fields"]["message"] == "sync job schedule changed")
        .ok_or("change log missing")?;
    let spans = line["spans"].as_array().ok_or("request span missing")?;
    let span = spans
        .iter()
        .find(|span| span["name"] == "log_ctx")
        .ok_or("log context missing")?;
    assert_eq!(span["correlation_id"], "synthetic-correlation");
    assert_eq!(span["tenant_id"], "synthetic-tenant");
    Ok(())
}
