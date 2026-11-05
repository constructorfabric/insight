use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

use super::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn stored() -> Model {
    Model {
        connector: "example-tracker".to_owned(),
        source_id: "main".to_owned(),
        crons: serde_json::json!(["0 3 * * *"]),
        revision: 1,
        updated_by: Uuid::from_u128(1),
        updated_at: chrono::DateTime::UNIX_EPOCH,
    }
}

#[tokio::test]
async fn changed_crons_at_the_maximum_revision_never_update() -> R {
    let instance = Instance::parse("example-tracker", "main")?;
    let mut row = stored();
    row.revision = u64::MAX;
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([vec![row]])
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }])
        .into_connection();
    let result = write(
        &db,
        &instance,
        Revision(u64::MAX),
        vec!["0 6 * * *".to_owned()],
        Uuid::from_u128(2),
    )
    .await;
    assert!(
        matches!(result, Err(Error::RevisionExhausted)),
        "{result:?}"
    );
    let log = format!("{:?}", db.into_transaction_log());
    assert!(!log.contains("UPDATE"), "{log}");
    Ok(())
}

#[tokio::test]
async fn clearing_revision_zero_never_writes_and_refuses_an_existing_row() -> R {
    let instance = Instance::parse("example-tracker", "main")?;
    for exists in [false, true] {
        let rows = if exists { vec![stored()] } else { Vec::new() };
        let db = MockDatabase::new(DatabaseBackend::MySql)
            .append_query_results([rows])
            .into_connection();
        let result = write(&db, &instance, Revision(0), vec![], Uuid::from_u128(1)).await;
        if exists {
            assert!(matches!(result, Err(Error::Stale(_))));
        } else {
            assert!(result?.is_none());
        }
        let log = format!("{:?}", db.into_transaction_log());
        assert!(
            !log.contains("INSERT") && !log.contains("UPDATE") && !log.contains("DELETE"),
            "{log}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn compare_and_set_real_changes_only_update_the_expected_revision() -> R {
    let instance = Instance::parse("example-tracker", "main")?;
    for crons in [vec!["0 6 * * *".to_owned()], Vec::new()] {
        for rows_affected in [0, 1] {
            let db = MockDatabase::new(DatabaseBackend::MySql)
                .append_query_results([vec![stored()], vec![stored()]])
                .append_exec_results([MockExecResult {
                    last_insert_id: 0,
                    rows_affected,
                }])
                .into_connection();
            let result = write(
                &db,
                &instance,
                Revision(1),
                crons.clone(),
                Uuid::from_u128(2),
            )
            .await;
            if rows_affected == 0 {
                assert!(matches!(result, Err(Error::Stale(_))));
            } else {
                let row = result?.ok_or("written row missing")?;
                assert_eq!(row.revision, 2);
                assert_eq!(row.updated_by, Uuid::from_u128(2));
                assert_eq!(row.crons, serde_json::json!(crons));
            }
            let log = format!("{:?}", db.into_transaction_log());
            assert!(
                log.contains("`connector_schedules`.`revision` = ?"),
                "{log}"
            );
            assert!(
                log.contains("`connector_schedules`.`connector` = ?"),
                "{log}"
            );
            assert!(
                log.contains("`connector_schedules`.`source_id` = ?"),
                "{log}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn unchanged_schedules_preserve_the_row_without_writing() -> R {
    let instance = Instance::parse("example-tracker", "main")?;
    for crons in [serde_json::json!([" 0\t3 * * * "]), serde_json::json!([])] {
        let mut row = stored();
        row.crons = crons;
        row.revision = u64::MAX;
        let requested = if row.crons == serde_json::json!([]) {
            vec![]
        } else {
            vec!["0 3 * * *".to_owned()]
        };
        let db = MockDatabase::new(DatabaseBackend::MySql)
            .append_query_results([vec![row.clone()]])
            .into_connection();
        let result = write(
            &db,
            &instance,
            Revision(u64::MAX),
            requested,
            Uuid::from_u128(2),
        )
        .await?
        .ok_or("missing unchanged row")?;
        assert_eq!(result, row);
        let log = format!("{:?}", db.into_transaction_log());
        for mutation in ["INSERT", "UPDATE", "DELETE"] {
            assert!(!log.contains(mutation), "{log}");
        }
    }
    Ok(())
}

#[tokio::test]
async fn stale_revisions_are_rejected_even_when_the_schedule_is_unchanged() -> R {
    let instance = Instance::parse("example-tracker", "main")?;
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([vec![stored()]])
        .into_connection();
    let result = write(
        &db,
        &instance,
        Revision(0),
        vec!["0 3 * * *".to_owned()],
        Uuid::from_u128(2),
    )
    .await;
    assert!(matches!(result, Err(Error::Stale(1))));
    Ok(())
}

#[tokio::test]
async fn a_noop_snapshot_emits_no_write_beside_a_concurrent_real_change() -> R {
    let instance = Instance::parse("example-tracker", "main")?;
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([vec![stored()], vec![stored()]])
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }])
        .into_connection();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let noop = AFTER_WRITE_READ.scope(
        barrier.clone(),
        write(
            &db,
            &instance,
            Revision(1),
            vec!["0 3 * * *".to_owned()],
            Uuid::from_u128(2),
        ),
    );
    let change = AFTER_WRITE_READ.scope(
        barrier,
        write(
            &db,
            &instance,
            Revision(1),
            vec!["0 9 * * *".to_owned()],
            Uuid::from_u128(3),
        ),
    );
    let (noop, change) = Box::pin(tokio::time::timeout(
        std::time::Duration::from_secs(5),
        async { tokio::join!(noop, change) },
    ))
    .await?;
    let noop = noop?.ok_or("no-op snapshot missing")?;
    let change = change?.ok_or("real change missing")?;
    assert_eq!(noop.revision, 1);
    assert_eq!(noop.updated_by, Uuid::from_u128(1));
    assert_eq!(change.revision, 2);
    assert_eq!(change.crons, serde_json::json!(["0 9 * * *"]));
    let log = format!("{:?}", db.into_transaction_log());
    assert_eq!(log.matches("UPDATE").count(), 1, "{log}");
    Ok(())
}
