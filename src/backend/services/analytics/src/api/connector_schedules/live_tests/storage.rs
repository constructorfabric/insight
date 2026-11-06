use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use super::*;

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn migrations_expose_only_the_versioned_request_columns_and_reject_invalid_acknowledgements()
-> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let (connector, _) = identity();
    let manager = SchemaManager::new(&db);
    Migration.up(&manager).await?;
    assert_view_contract(&db, &connector).await?;
    assert_invalid_records(&db, &connector).await?;
    assert_seeded_records(&db, &connector).await?;
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::MySql,
        "DELETE FROM connector_schedule_acknowledgements WHERE connector = ?",
        [connector.clone().into()],
    ))
    .await?;
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

async fn assert_invalid_records(db: &DatabaseConnection, connector: &str) -> R {
    for (
        name,
        applied_crons,
        deployment_crons,
        applied_origin,
        deployment_origin,
        applied,
        attempted,
    ) in [
        ("applied array", "{}", "[]", "api", "default", "1", "1"),
        ("deployment array", "[]", "{}", "api", "default", "1", "1"),
        ("applied origin", "[]", "[]", "unknown", "default", "1", "1"),
        (
            "applied origin casing",
            "[]",
            "[]",
            "API",
            "default",
            "1",
            "1",
        ),
        ("deployment origin", "[]", "[]", "api", "unknown", "1", "1"),
        (
            "deployment origin casing",
            "[]",
            "[]",
            "api",
            "DEFAULT",
            "1",
            "1",
        ),
        (
            "fallback cannot be an override",
            "[]",
            "[]",
            "api",
            "api",
            "1",
            "1",
        ),
        ("revision order", "[]", "[]", "api", "default", "2", "1"),
        (
            "negative applied revision",
            "[]",
            "[]",
            "api",
            "default",
            "-1",
            "1",
        ),
        (
            "negative attempted revision",
            "[]",
            "[]",
            "api",
            "default",
            "1",
            "-1",
        ),
    ] {
        let sql = format!("INSERT INTO connector_schedule_acknowledgements
            (connector, source_id, applied_crons, deployment_crons, applied_origin, deployment_origin, applied_revision, attempted_revision, attempted_at)
            VALUES (?, 'main', ?, ?, ?, ?, {applied}, {attempted}, UTC_TIMESTAMP(3))");
        let result = db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::MySql,
                sql,
                [
                    connector.to_owned().into(),
                    applied_crons.into(),
                    deployment_crons.into(),
                    applied_origin.into(),
                    deployment_origin.into(),
                ],
            ))
            .await;
        assert!(result.is_err(), "should reject: {name}");
    }
    db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::MySql,
        "INSERT INTO connector_schedule_acknowledgements (connector, source_id, deployment_crons, deployment_origin, attempted_revision, attempted_at)
         VALUES (?, 'main', '[]', 'default', 0, UTC_TIMESTAMP(3))", [connector.to_owned().into()]
    )).await?;
    Ok(())
}

async fn assert_seeded_records(db: &DatabaseConnection, connector: &str) -> R {
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../tests/stand/fixtures/connector-schedule-acknowledgements.sql"
    ))
    .replace("stand-scratch-connector-schedules", connector);
    db.execute_unprepared(&fixture).await?;
    db.execute_unprepared(&fixture).await?;
    let count = db
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::MySql,
            "SELECT count(*) AS total FROM connector_schedule_acknowledgements WHERE connector = ?",
            [connector.to_owned().into()],
        ))
        .await?
        .ok_or("count missing")?;
    assert_eq!(count.try_get::<i64>("", "total")?, 3);

    let mut health: crate::domain::connector_health::ConnectorHealthResponse =
        serde_json::from_value(json!({
            "as_of": "2026-01-15T13:00:00Z", "history_available": true,
            "connectors": [
                {"connector": connector, "source_id": "applied", "configured": true},
                {"connector": connector, "source_id": "error", "configured": true},
                {"connector": connector, "source_id": "main", "configured": true}
            ]
        }))?;
    crate::domain::connector_schedules::enrich_health(db, &mut health).await;
    let health = serde_json::to_value(health)?;
    for (index, expected) in [(0, "applied"), (1, "error"), (2, "pending")] {
        assert_eq!(
            health["connectors"][index]["schedule_request"]["status"],
            expected
        );
    }
    assert_eq!(
        health["connectors"][1]["schedule_request"]["error"]["message"],
        "synthetic readback failure"
    );
    Ok(())
}
