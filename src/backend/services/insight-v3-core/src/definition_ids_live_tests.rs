use sea_orm::{ConnectionTrait as _, DatabaseConnection, DbBackend, FromQueryResult, Statement};
use sea_orm_migration::{MigratorTrait as _, SchemaManager};

use crate::live_mariadb::{database_or_skip, unique};
use crate::store::definitions::migration::Migrator;
use crate::store::definitions::migration::m20261021_000009_definition_ids::key_by_id;

#[derive(Debug, FromQueryResult, PartialEq, Eq)]
struct KeyedRow {
    id: String,
    name: String,
}

#[derive(Debug, FromQueryResult)]
struct TotalRow {
    total: i64,
}

async fn run(db: &DatabaseConnection, sql: &str) {
    db.execute_unprepared(sql)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
}

async fn counted(db: &DatabaseConnection, sql: &str) -> i64 {
    TotalRow::find_by_statement(Statement::from_string(DbBackend::MySql, sql))
        .one(db)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .map_or(0, |row| row.total)
}

#[tokio::test]
async fn a_table_keyed_by_name_is_keyed_by_that_name_as_its_id_and_a_rerun_changes_nothing() {
    let Some(db) = database_or_skip().await else {
        return;
    };
    let table = unique("legacy");
    let child = format!("{table}_child");
    run(
        &db,
        &format!(
            "CREATE TABLE {table} (name VARCHAR(128) NOT NULL PRIMARY KEY, body JSON NOT NULL, updated_at DATETIME(6) NOT NULL) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci"
        ),
    )
    .await;
    run(
        &db,
        &format!(
            "CREATE TABLE {child} (holder VARCHAR(128) NOT NULL, CONSTRAINT {child}_holder FOREIGN KEY (holder) REFERENCES {table} (name) ON DELETE CASCADE) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci"
        ),
    )
    .await;
    run(
        &db,
        &format!(
            "INSERT INTO {table} (name, body, updated_at) VALUES ('commits_per_day', '{{}}', UTC_TIMESTAMP(6)), ('lines', '{{}}', UTC_TIMESTAMP(6))"
        ),
    )
    .await;
    run(
        &db,
        &format!("INSERT INTO {child} (holder) VALUES ('commits_per_day')"),
    )
    .await;

    let manager = SchemaManager::new(&db);
    for _ in 0..2 {
        key_by_id(&manager, &table)
            .await
            .unwrap_or_else(|error| panic!("the step must apply twice over: {error}"));
    }
    let rows = KeyedRow::find_by_statement(Statement::from_string(
        DbBackend::MySql,
        format!("SELECT id, name FROM {table} ORDER BY id"),
    ))
    .all(&db)
    .await
    .unwrap_or_else(|error| panic!("{error}"));
    run(
        &db,
        &format!("DELETE FROM {table} WHERE id = 'commits_per_day'"),
    )
    .await;
    let orphans = counted(&db, &format!("SELECT COUNT(*) AS total FROM {child}")).await;
    let indexed = counted(
        &db,
        &format!(
            "SELECT COUNT(*) AS total FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = '{table}' AND INDEX_NAME = '{table}_name'"
        ),
    )
    .await;

    run(&db, &format!("DROP TABLE {child}")).await;
    run(&db, &format!("DROP TABLE {table}")).await;
    assert_eq!(
        rows,
        [
            KeyedRow {
                id: "commits_per_day".to_owned(),
                name: "commits_per_day".to_owned(),
            },
            KeyedRow {
                id: "lines".to_owned(),
                name: "lines".to_owned(),
            },
        ]
    );
    assert_eq!(orphans, 0, "the foreign key must follow the renamed key");
    assert_eq!(indexed, 1, "the name is indexed once");
}

#[tokio::test]
async fn every_definition_table_is_keyed_by_id_and_the_migrator_runs_again_as_a_no_op() {
    let Some(db) = database_or_skip().await else {
        return;
    };

    Migrator::up(&db, None)
        .await
        .unwrap_or_else(|error| panic!("a migrated schema migrates again: {error}"));

    for table in ["metrics", "widgets", "dashboards"] {
        let columns = counted(
            &db,
            &format!(
                "SELECT COUNT(*) AS total FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = '{table}' AND COLUMN_NAME IN ('id', 'name')"
            ),
        )
        .await;
        let keyed = counted(
            &db,
            &format!(
                "SELECT COUNT(*) AS total FROM information_schema.KEY_COLUMN_USAGE WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = '{table}' AND CONSTRAINT_NAME = 'PRIMARY' AND COLUMN_NAME = 'id'"
            ),
        )
        .await;

        assert_eq!((columns, keyed), (2, 1), "{table}");
    }
}
