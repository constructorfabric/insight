use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use super::*;

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn migration_preserves_the_frozen_view_and_enforces_requests() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, _) = identity();
    Migration.up(&SchemaManager::new(&db)).await?;
    assert_view_contract(&db, &connector).await?;
    assert_invalid_requests(&db, &connector).await?;
    clear(&db, &connector).await
}

async fn assert_view_contract(db: &DatabaseConnection, connector: &str) -> R {
    let columns = db.query_all_raw(Statement::from_string(DatabaseBackend::MySql,
        "SELECT COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'connector_schedule_requests_v1' ORDER BY ORDINAL_POSITION"
    )).await?;
    let columns: Vec<String> = columns
        .iter()
        .map(|row| row.try_get("", "COLUMN_NAME"))
        .collect::<Result<_, _>>()?;
    assert_eq!(columns, ["connector", "source_id", "revision", "crons"]);
    let view = db.query_one_raw(Statement::from_string(DatabaseBackend::MySql,
        "SELECT SECURITY_TYPE FROM information_schema.VIEWS WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'connector_schedule_requests_v1'"
    )).await?.ok_or("view missing")?;
    assert_eq!(view.try_get::<String>("", "SECURITY_TYPE")?, "DEFINER");

    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::MySql,
        "INSERT INTO connector_schedules VALUES (?, 'main', '[]', 1, ?, UTC_TIMESTAMP(3))",
        [connector.to_owned().into(), CALLER.into()],
    ))
    .await?;
    let cleared = db.query_one_raw(Statement::from_sql_and_values(DatabaseBackend::MySql,
        "SELECT crons, revision FROM connector_schedule_requests_v1 WHERE connector = ? AND source_id = 'main'",
        [connector.to_owned().into()]
    )).await?.ok_or("view omitted clear")?;
    assert_eq!(
        cleared.try_get::<serde_json::Value>("", "crons")?,
        json!([])
    );
    assert_eq!(cleared.try_get::<u64>("", "revision")?, 1);

    Ok(())
}

async fn assert_invalid_requests(db: &DatabaseConnection, connector: &str) -> R {
    for (source, crons, revision) in [("zero-revision", "[]", 0), ("object-crons", "{}", 1)] {
        let result = db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::MySql,
                "INSERT INTO connector_schedules VALUES (?, ?, ?, ?, ?, UTC_TIMESTAMP(3))",
                [
                    connector.to_owned().into(),
                    source.into(),
                    crons.into(),
                    revision.into(),
                    CALLER.into(),
                ],
            ))
            .await;
        assert!(result.is_err(), "should reject: {source}");
        db.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::MySql,
            "INSERT INTO connector_schedules VALUES (?, ?, '[]', 1, ?, UTC_TIMESTAMP(3))",
            [connector.to_owned().into(), source.into(), CALLER.into()],
        ))
        .await?;
    }
    Ok(())
}
