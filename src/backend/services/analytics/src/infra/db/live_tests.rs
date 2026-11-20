use std::sync::LazyLock;

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;

use super::{assert_schema_compatible, connect};

static LEDGER_MUTEX: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

const ENV_VAR: &str = "INTEGRATION_TESTS_MARIADB_URL";

const FUTURE_VERSION: &str = "m99999999_000001_from_a_newer_release";

fn url_or_skip() -> Option<String> {
    let Ok(url) = std::env::var(ENV_VAR) else {
        eprintln!("skipping: {ENV_VAR} not set");
        return None;
    };
    Some(url)
}

async fn count(db: &sea_orm::DatabaseConnection, sql: &str) -> anyhow::Result<i64> {
    let row = db
        .query_one_raw(Statement::from_string(DbBackend::MySql, sql))
        .await?
        .ok_or_else(|| anyhow::anyhow!("count query returned no row"))?;
    Ok(row.try_get_by_index::<i64>(0)?)
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn migrator_converges_when_re_run() -> anyhow::Result<()> {
    let Some(url) = url_or_skip() else {
        return Ok(());
    };
    let _serialized = LEDGER_MUTEX.lock().await;
    let db = connect(&url).await?;

    insight_migration::with_migration_session(
        &url,
        super::MIGRATION_LOCK,
        super::MIGRATION_LOCK_TIMEOUT,
        |session| async move { Ok(crate::migration::Migrator::up(&session, None).await?) },
    )
    .await?;

    let applied = count(&db, "SELECT COUNT(*) FROM seaql_migrations").await?;
    let embedded = i64::try_from(crate::migration::Migrator::migrations().len())?;
    assert_eq!(
        applied, embedded,
        "ledger must hold exactly the embedded set"
    );
    assert_schema_compatible(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn newer_ledger_rows_do_not_block_boot() -> anyhow::Result<()> {
    let Some(url) = url_or_skip() else {
        return Ok(());
    };
    let _serialized = LEDGER_MUTEX.lock().await;
    let db = connect(&url).await?;

    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        "INSERT INTO seaql_migrations (version, applied_at) VALUES (?, 0)",
        [FUTURE_VERSION.into()],
    ))
    .await?;

    let tolerated = assert_schema_compatible(&db).await;

    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        "DELETE FROM seaql_migrations WHERE version = ?",
        [FUTURE_VERSION.into()],
    ))
    .await?;

    tolerated?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn schema_behind_the_build_fails_the_boot_gate() -> anyhow::Result<()> {
    let Some(url) = url_or_skip() else {
        return Ok(());
    };
    let _serialized = LEDGER_MUTEX.lock().await;
    let db = connect(&url).await?;

    let newest = crate::migration::Migrator::migrations()
        .last()
        .map(|m| m.name().to_owned())
        .ok_or_else(|| anyhow::anyhow!("no embedded migrations"))?;

    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        "DELETE FROM seaql_migrations WHERE version = ?",
        [newest.as_str().into()],
    ))
    .await?;

    let verdict = assert_schema_compatible(&db).await;

    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        "INSERT INTO seaql_migrations (version, applied_at) VALUES (?, 0)",
        [newest.as_str().into()],
    ))
    .await?;

    let Err(err) = verdict else {
        panic!("a missing embedded migration must fail the boot gate");
    };
    assert!(
        err.to_string().contains(&newest),
        "the error must name the missing migration, got: {err}"
    );
    assert_schema_compatible(&db).await?;
    Ok(())
}
