use std::time::Duration;

use super::*;

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn instance_locks_serialize_connections_and_a_delayed_old_save_cannot_win() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = identity();
    let instance = crate::domain::connector_schedules::Instance::parse(&connector, "main")?;
    let guard = crate::domain::connector_schedules::lock::acquire(&db, &instance).await?;
    let waiting = crate::domain::connector_schedules::lock::acquire(&db, &instance);
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut waiting)
            .await
            .is_err()
    );
    guard.release().await?;
    waiting.await?.release().await?;

    let history = Mock::new();
    let h = harness(db.clone(), history.url(), context("user", &[])?, true).await?;
    assert_eq!(
        send((&h, &history), "PUT", &uri, Some("\"0\""), &["0 6 * * *"])
            .await?
            .0,
        StatusCode::OK
    );
    for method in ["PUT", "DELETE"] {
        let (status, _) = send((&h, &history), method, &uri, Some("\"0\""), &["0 3 * * *"]).await?;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    }
    let row = Entity::find_by_id((connector.clone(), "main".to_owned()))
        .one(&db)
        .await?
        .ok_or("schedule")?;
    assert_eq!(row.crons, json!(["0 6 * * *"]));
    clear(&db, &connector).await
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn an_older_handler_delayed_before_the_lock_cannot_overwrite_a_newer_save() -> R {
    use crate::infra::{
        cluster_stub::{Scenario, Stub},
        identity::IdentityClient,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Notify;

    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = identity();
    let stub = Stub::start(Scenario::Accept, &connector, "main").await?;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let identity_app = axum::Router::new().route(
        "/v1/me",
        axum::routing::get({
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            move || {
                let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
                let entered = Arc::clone(&entered);
                let release = Arc::clone(&release);
                async move {
                    if first {
                        entered.notify_one();
                        release.notified().await;
                    }
                    axum::Json(
                        json!({"roles":[{"role_id":"a4d11000-0000-4000-8000-000000000001"}]}),
                    )
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let identity = IdentityClient::new(&format!("http://{}", listener.local_addr()?))?;
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, identity_app).await;
    });
    let history = Mock::new();
    for _ in 0..2 {
        history.add(handlers::provide([configured_instance_record()]));
    }
    let state = Arc::new(crate::api::AppState {
        db: db.clone(),
        ch: insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            history.url(),
            "insight",
        )),
        identity,
        cluster: stub.cluster.clone(),
        schedule_slots: tokio::sync::Semaphore::new(4),
    });
    let app = super::super::support::router(state, context("user", &[])?);
    let old_request = request("PUT", &uri, Some("\"0\""), &json!({"crons":["0 3 * * *"]}))?;
    let older = tokio::spawn(app.clone().oneshot(old_request));
    entered.notified().await;
    let newer = app
        .oneshot(request(
            "PUT",
            &uri,
            Some("\"0\""),
            &json!({"crons":["0 6 * * *"]}),
        )?)
        .await?;
    assert_eq!(newer.status(), StatusCode::OK);
    assert_eq!(body(newer).await?["status"], "applied");
    release.notify_one();
    let older = older.await??;
    assert_eq!(older.status(), StatusCode::PRECONDITION_FAILED);
    assert_eq!(older.headers()["etag"], "\"1\"");
    assert_eq!(stub.patches().await.len(), 1);
    assert_eq!(
        stub.object().await["spec"]["schedules"],
        json!(["0 6 * * *"])
    );
    server.abort();
    clear(&db, &connector).await
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn reads_do_not_wait_for_the_instance_lock_and_saves_have_bounded_waits() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, uri) = identity();
    let history = Mock::new();
    let h = harness(db.clone(), history.url(), context("user", &[])?, true).await?;
    let instance = crate::domain::connector_schedules::Instance::parse(&connector, "main")?;
    let guard = crate::domain::connector_schedules::lock::acquire(&db, &instance).await?;
    let read = tokio::time::timeout(
        Duration::from_secs(1),
        send((&h, &history), "GET", &uri, None, &[]),
    )
    .await??;
    assert_eq!(read.0, StatusCode::OK);
    let started = tokio::time::Instant::now();
    let save = tokio::time::timeout(
        Duration::from_secs(6),
        send((&h, &history), "PUT", &uri, Some("\"0\""), &["0 6 * * *"]),
    )
    .await??;
    assert_eq!(save.0, StatusCode::SERVICE_UNAVAILABLE);
    assert!(started.elapsed() >= Duration::from_secs(5));
    guard.release().await?;
    assert!(
        save.1["detail"]
            .as_str()
            .ok_or("detail")?
            .contains("not changed")
    );
    assert!(
        save.1["detail"]
            .as_str()
            .ok_or("detail")?
            .contains("try again")
    );
    let _slots = h.state.schedule_slots.acquire_many(4).await?;
    let read = tokio::time::timeout(
        Duration::from_secs(1),
        send((&h, &history), "GET", &uri, None, &[]),
    )
    .await??;
    assert_eq!(read.0, StatusCode::OK);
    let started = tokio::time::Instant::now();
    let save = tokio::time::timeout(
        Duration::from_secs(6),
        send((&h, &history), "PUT", &uri, Some("\"0\""), &["0 6 * * *"]),
    )
    .await??;
    assert_eq!(save.0, StatusCode::SERVICE_UNAVAILABLE);
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert!(
        Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?
            .is_none()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL and permission to kill own sessions"]
async fn a_failed_lock_release_does_not_replace_a_finished_save_answer() -> R {
    use crate::domain::connector_schedules::save_hooks;
    use crate::infra::cluster_stub::Scenario;
    use sea_orm::{ConnectionTrait, DbBackend, Statement};

    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
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
    let gate = save_hooks::install(&connector, "release");
    let handler = tokio::spawn(h.app.clone().oneshot(request(
        "PUT",
        &uri,
        Some("\"0\""),
        &json!({"crons":["0 6 * * *"]}),
    )?));
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified()).await?;
    let owner = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            "SELECT CAST(IS_USED_LOCK(SHA2(CONCAT(DATABASE(), '/', ?),256)) AS UNSIGNED) AS owner",
            [format!("{connector}/main").into()],
        ))
        .await?
        .ok_or("lock owner")?
        .try_get::<u64>("", "owner")?;
    db.execute_raw(Statement::from_string(
        DbBackend::MySql,
        format!("KILL {owner}"),
    ))
    .await?;
    gate.resume.notify_one();
    let response = handler.await??;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(response).await?["status"], "applied");
    assert_eq!(
        h.workflow.object().await["spec"]["schedules"],
        json!(["0 6 * * *"])
    );
    assert_eq!(
        Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?
            .ok_or("row")?
            .crons,
        json!(["0 6 * * *"])
    );
    save_hooks::remove(&connector);
    clear(&db, &connector).await
}
