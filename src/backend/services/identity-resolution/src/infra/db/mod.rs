//! `MariaDB` connection.
//!
//! **Self-managed `SeaORM` pool — we deliberately do NOT use the toolkit `db`
//! capability** (same as the analytics gear). The identity queries need SQL that
//! `cf-gears-toolkit-db` (v0.8.4) can neither express via its scoped
//! entity-builder nor run as raw SQL (it intentionally exposes no raw-SQL path —
//! `DbConn`/`DbTx` are builder-only). Specifically:
//!   * window functions (`ROW_NUMBER()` / `LEAD() OVER (…)`) — the resolver reads
//!     and the SCD2 `account_person_map` / `org_chart` rebuilds;
//!   * `WITH RECURSIVE` — the org-subchart / visibility traversals;
//!   * atomic conditional DML with a correlated subquery — the role in-use and
//!     last-admin lockout guards.
//!
//! See constructorfabric/gears-rust#4239 for the capability request.
//!
//! All SQL here is **verbatim from the .NET service** (cutover parity). It is
//! injection-safe despite being raw: every value is a **bound parameter**
//! (`Statement::from_sql_and_values`, no string interpolation) and the tenant is
//! always pinned in the `WHERE`. The `identity` database is owned by .NET today.

pub mod bootstrap;
pub mod entities;
pub mod ops_repo;
pub mod person_roles_repo;
pub mod persons_repo;
pub mod roles_repo;
pub mod seed_repo;
pub mod sql_named;
pub mod subchart_repo;
pub mod visibility_repo;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};

/// Connect to `MariaDB` and return a connection pool.
///
/// # Errors
///
/// Returns an error if the connection cannot be established.
pub async fn connect(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut opts = ConnectOptions::new(database_url);
    opts.max_connections(10)
        .min_connections(2)
        .sqlx_logging(false);

    let db = Database::connect(opts).await?;
    tracing::info!("connected to MariaDB");
    Ok(db)
}

/// Connect with a SINGLE pooled connection — for the `migrate` subcommand.
///
/// The migration run is guarded by a `GET_LOCK` advisory lock, which is
/// session-scoped: lock, DDL, and release must all execute on the same
/// connection, so the pool is capped at one.
///
/// # Errors
///
/// Returns an error if the connection cannot be established.
pub async fn connect_single(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut opts = ConnectOptions::new(database_url);
    opts.max_connections(1)
        .min_connections(1)
        .sqlx_logging(false);

    let db = Database::connect(opts).await?;
    tracing::info!("connected to MariaDB (single-connection migrate session)");
    Ok(db)
}

/// Name of the cross-process advisory lock serializing schema migration runs.
const MIGRATION_LOCK: &str = "identity_resolution_migrations";
/// How long a second migrator waits for the lock before giving up (seconds).
const MIGRATION_LOCK_TIMEOUT_SECS: i32 = 300;

/// Run pending migrations under a `GET_LOCK` advisory lock.
///
/// The lock serializes concurrent migrators (two Rust initContainers, or a
/// Rust migrate racing the frozen .NET `DbUp` startup pass) — MariaDB DDL is
/// not transactional, so without it two racers could double-apply a pending
/// script. Call with a [`connect_single`] connection: `GET_LOCK` is
/// session-scoped and must share the session with the DDL.
///
/// # Errors
///
/// Returns an error if the lock cannot be acquired within the timeout or a
/// migration fails.
pub async fn run_migrations(db: &DatabaseConnection) -> anyhow::Result<()> {
    use sea_orm::{ConnectionTrait, DbBackend, Statement};
    use sea_orm_migration::MigratorTrait;

    let acquired: Option<i8> = db
        .query_one(Statement::from_sql_and_values(
            DbBackend::MySql,
            "SELECT GET_LOCK(?, ?)",
            [MIGRATION_LOCK.into(), MIGRATION_LOCK_TIMEOUT_SECS.into()],
        ))
        .await?
        .map(|r| r.try_get_by_index::<Option<i8>>(0))
        .transpose()?
        .flatten();
    anyhow::ensure!(
        acquired == Some(1),
        "could not acquire the `{MIGRATION_LOCK}` advisory lock within \
         {MIGRATION_LOCK_TIMEOUT_SECS}s — is another migrate run stuck?"
    );

    let result = crate::migration::Migrator::up(db, None).await;

    // Best-effort release either way; the lock also dies with the session.
    let _ = db
        .execute(Statement::from_sql_and_values(
            DbBackend::MySql,
            "SELECT RELEASE_LOCK(?)",
            [MIGRATION_LOCK.into()],
        ))
        .await;

    result?;
    tracing::info!("migrations applied");
    Ok(())
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, DbBackend, Statement};
    use sea_orm_migration::MigratorTrait;

    use super::*;
    use crate::config::GearConfig;

    async fn count(db: &DatabaseConnection, sql: &str) -> anyhow::Result<i64> {
        let row = db
            .query_one(Statement::from_string(DbBackend::MySql, sql))
            .await?
            .ok_or_else(|| anyhow::anyhow!("count query returned no row"))?;
        Ok(row.try_get_by_index::<i64>(0)?)
    }

    /// Live migration + bootstrap test against the CI-provisioned MariaDB
    /// (`INTEGRATION_TESTS_MARIADB_URL`; the CI job applies migrations once
    /// via the CLI before tests, so the first `run_migrations` here is
    /// already a re-run). Skips cleanly when the env var is unset.
    #[tokio::test]
    async fn migrations_and_bootstrap_are_idempotent_against_live_mariadb() -> anyhow::Result<()> {
        let Ok(url) = std::env::var("INTEGRATION_TESTS_MARIADB_URL") else {
            eprintln!("skip: set INTEGRATION_TESTS_MARIADB_URL to run");
            return Ok(());
        };
        let db = connect_single(&url).await?;

        run_migrations(&db).await?;
        run_migrations(&db).await?;

        let applied = count(&db, "SELECT COUNT(*) FROM seaql_migrations").await?;
        let embedded = i64::try_from(crate::migration::Migrator::migrations().len())?;
        assert_eq!(
            applied, embedded,
            "ledger must hold exactly the embedded set"
        );

        // Crash-recovery regression (012): a migrator killed between the DROP
        // and ADD CONSTRAINT statements leaves the constraint absent — a
        // re-run must converge, not fail on the unconditional DROP.
        db.execute(Statement::from_string(
            DbBackend::MySql,
            "ALTER TABLE org_chart DROP CONSTRAINT IF EXISTS chk_no_self_loop",
        ))
        .await?;
        db.execute(Statement::from_string(
            DbBackend::MySql,
            "DELETE FROM seaql_migrations WHERE version = 'm20260724_000012_org_chart_nullable_parent'",
        ))
        .await?;
        run_migrations(&db).await?;
        let checks = count(
            &db,
            "SELECT COUNT(*) FROM information_schema.CHECK_CONSTRAINTS \
             WHERE CONSTRAINT_NAME = 'chk_no_self_loop'",
        )
        .await?;
        assert_eq!(checks, 1, "012 re-run must restore the constraint");

        // Bootstrap admin: two runs, one active assignment.
        let cfg = GearConfig {
            tenant_default_id: "3e1d5a65-434c-95b4-8c1b-eb8f53a39bab".to_owned(),
            bootstrap_admin_person_id: "019e27bc-dec0-7626-81a9-c5524662a6a9".to_owned(),
            ..GearConfig::default()
        };
        bootstrap::bootstrap_admin(&db, &cfg).await?;
        bootstrap::bootstrap_admin(&db, &cfg).await?;
        let admins = count(
            &db,
            "SELECT COUNT(*) FROM person_roles \
             WHERE reason = 'bootstrap' AND valid_to IS NULL",
        )
        .await?;
        assert_eq!(admins, 1, "bootstrap must be idempotent");
        Ok(())
    }
}
