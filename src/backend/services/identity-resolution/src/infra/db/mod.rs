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
//! always pinned in the `WHERE`. The `identity` database schema is owned by THIS
//! service's migrator — see the ownership-transfer docs in `crate::migration`.

pub mod bootstrap;
pub mod entities;
pub mod ops_repo;
pub mod person_listing;
pub mod person_roles_repo;
pub mod persons_log_repo;
pub mod persons_repo;
pub mod resolution_repo;
pub mod roles_repo;
pub mod seed_repo;
pub mod sql_named;
pub mod subchart_repo;
pub mod visibility_repo;

#[cfg(test)]
mod binding_reads_live_tests;
#[cfg(test)]
mod roster_live_tests;
#[cfg(test)]
pub(crate) mod test_fixture;
#[cfg(test)]
mod visible_set_live_tests;

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

/// Prefix of the per-tenant advisory lock serializing persons-seed runs.
/// Cross-process AND cross-instance: the lock lives on the MariaDB server, so
/// a cron Job, a manual Job, and a second Insight instance sharing the same
/// database all serialize through it.
const SEED_LOCK_PREFIX: &str = "persons-seed:";

/// RAII holder of the per-tenant persons-seed advisory lock.
///
/// The lock is `GET_LOCK`-session-scoped, and this guard OWNS the dedicated
/// single-connection session it was acquired on — so the lock's lifetime is
/// tied to the guard's scope by construction. Every exit path is covered:
///   * happy path — [`SeedLockGuard::release`] issues `RELEASE_LOCK`
///     (fastest handover to the next run);
///   * early return / future cancellation — the guard drops, the session
///     closes, MariaDB releases the lock server-side;
///   * process crash (panic, OOM-kill, node death) — the TCP session dies
///     and MariaDB releases the lock the same way.
///
/// A stale lock is therefore impossible; no explicit `Drop` impl is needed —
/// dropping the owned connection IS the release.
pub struct SeedLockGuard {
    conn: DatabaseConnection,
    tenant_id: uuid::Uuid,
}

impl SeedLockGuard {
    /// Try to take the per-tenant lock without waiting (`GET_LOCK` timeout 0
    /// — a concurrent run fails fast instead of queueing a stale re-run
    /// behind the active one). Opens its own single-connection session (see
    /// [`connect_single`]; a pooled connection could be swapped mid-run,
    /// silently dropping the session-scoped lock). Returns `None` when
    /// another run holds the lock.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection or the query fails.
    pub async fn try_acquire(
        database_url: &str,
        tenant_id: uuid::Uuid,
    ) -> anyhow::Result<Option<Self>> {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        let conn = connect_single(database_url).await?;
        let acquired: Option<i8> = conn
            .query_one(Statement::from_sql_and_values(
                DbBackend::MySql,
                "SELECT GET_LOCK(?, 0)",
                [format!("{SEED_LOCK_PREFIX}{tenant_id}").into()],
            ))
            .await?
            .map(|r| r.try_get_by_index::<Option<i8>>(0))
            .transpose()?
            .flatten();
        if acquired == Some(1) {
            Ok(Some(Self { conn, tenant_id }))
        } else {
            Ok(None)
        }
    }

    /// Explicit best-effort release (the happy path — hands the lock over
    /// without waiting for the session teardown). Consumes the guard; a
    /// failure is not worth propagating, dropping the session releases the
    /// lock anyway.
    pub async fn release(self) {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        let _ = self
            .conn
            .execute(Statement::from_sql_and_values(
                DbBackend::MySql,
                "SELECT RELEASE_LOCK(?)",
                [format!("{SEED_LOCK_PREFIX}{}", self.tenant_id).into()],
            ))
            .await;
    }
}

/// Name of the GLOBAL advisory lock serializing persons-sync runs (the sync
/// copies the whole log regardless of tenant, so unlike the seed there is no
/// per-tenant suffix). Same cross-process/cross-instance properties as the
/// seed lock; distinct from it so a sync never contends with a seed.
const SYNC_LOCK: &str = "persons-sync";

/// RAII holder of the persons-sync advisory lock — the global sibling of
/// [`SeedLockGuard`], with identical lifetime semantics (owns its dedicated
/// single-connection session; every exit path up to and including process
/// death releases the lock server-side).
pub struct SyncLockGuard {
    conn: DatabaseConnection,
}

impl SyncLockGuard {
    /// Try to take the global sync lock without waiting (`GET_LOCK` timeout 0
    /// — a concurrent run fails fast instead of publishing a stale snapshot
    /// after the active one). Returns `None` when another run holds it.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection or the query fails.
    pub async fn try_acquire(database_url: &str) -> anyhow::Result<Option<Self>> {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        let conn = connect_single(database_url).await?;
        let acquired: Option<i8> = conn
            .query_one(Statement::from_sql_and_values(
                DbBackend::MySql,
                "SELECT GET_LOCK(?, 0)",
                [SYNC_LOCK.into()],
            ))
            .await?
            .map(|r| r.try_get_by_index::<Option<i8>>(0))
            .transpose()?
            .flatten();
        if acquired == Some(1) {
            Ok(Some(Self { conn }))
        } else {
            Ok(None)
        }
    }

    /// Explicit best-effort release (the happy path); dropping the session
    /// releases the lock anyway.
    pub async fn release(self) {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        let _ = self
            .conn
            .execute(Statement::from_sql_and_values(
                DbBackend::MySql,
                "SELECT RELEASE_LOCK(?)",
                [SYNC_LOCK.into()],
            ))
            .await;
    }
}

/// Name of the cross-process advisory lock serializing schema migration runs.
const MIGRATION_LOCK: &str = "identity_resolution_migrations";
/// How long a second migrator waits for the lock before giving up (seconds).
const MIGRATION_LOCK_TIMEOUT_SECS: i32 = 300;

/// Run pending migrations AND the first-admin bootstrap under one `GET_LOCK`
/// advisory lock.
///
/// The lock serializes concurrent RUST migrators (two initContainers of two
/// replicas) — MariaDB DDL is not transactional, so without it two racers
/// could double-apply a pending script. It does NOT serialize against the
/// frozen .NET service: its `DbUp`/`BootstrapAdminRunner` startup pass takes
/// no advisory lock (there, safety rests on every script being idempotent,
/// and on single bootstrap OWNERSHIP — the umbrella render fails when both
/// services configure a bootstrap admin). The bootstrap runs INSIDE the same
/// critical section: its
/// `INSERT … WHERE NOT EXISTS` has no unique constraint backing the active
/// `(tenant, person, role)` triple, so two replicas racing after the lock was
/// released could insert two active bootstrap assignments. Call with a
/// [`connect_single`] connection: `GET_LOCK` is session-scoped and must share
/// the session with the DDL.
///
/// # Errors
///
/// Returns an error if the lock cannot be acquired within the timeout, a
/// migration fails, or the bootstrap fails.
pub async fn run_migrations(
    db: &DatabaseConnection,
    config: &crate::config::GearConfig,
) -> anyhow::Result<()> {
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

    let result = async {
        crate::migration::Migrator::up(db, None).await?;
        bootstrap::bootstrap_admin(db, config).await
    }
    .await;

    // Best-effort release either way; the lock also dies with the session.
    let _ = db
        .execute(Statement::from_sql_and_values(
            DbBackend::MySql,
            "SELECT RELEASE_LOCK(?)",
            [MIGRATION_LOCK.into()],
        ))
        .await;

    result?;
    tracing::info!("migrations + bootstrap applied");
    Ok(())
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, DbBackend, Statement};
    use sea_orm_migration::MigratorTrait;

    use super::*;
    use crate::config::GearConfig;

    async fn count(db: &DatabaseConnection, sql: &str) -> anyhow::Result<i64> {
        count_with(db, sql, []).await
    }

    async fn count_with(
        db: &DatabaseConnection,
        sql: &str,
        values: impl IntoIterator<Item = sea_orm::Value>,
    ) -> anyhow::Result<i64> {
        let row = db
            .query_one(Statement::from_sql_and_values(
                DbBackend::MySql,
                sql,
                values,
            ))
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
        let cfg = GearConfig {
            tenant_default_id: "3e1d5a65-434c-95b4-8c1b-eb8f53a39bab".to_owned(),
            bootstrap_admin_person_id: "019e27bc-dec0-7626-81a9-c5524662a6a9".to_owned(),
            ..GearConfig::default()
        };

        run_migrations(&db, &cfg).await?;
        run_migrations(&db, &cfg).await?;

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
        run_migrations(&db, &cfg).await?;
        let checks = count(
            &db,
            "SELECT COUNT(*) FROM information_schema.CHECK_CONSTRAINTS \
             WHERE CONSTRAINT_NAME = 'chk_no_self_loop'",
        )
        .await?;
        assert_eq!(checks, 1, "012 re-run must restore the constraint");

        // Bootstrap admin ran under the lock on EVERY run_migrations call
        // above (three so far) — exactly one active assignment must exist
        // for THIS test's (tenant, person, role) triple. Scoped, not a
        // global reason='bootstrap' count: a legitimate bootstrap for a
        // different tenant/person in the same database must not fail this.
        let tenant = uuid::Uuid::parse_str(&cfg.tenant_default_id)?;
        let person = uuid::Uuid::parse_str(&cfg.bootstrap_admin_person_id)?;
        let admins = count_with(
            &db,
            "SELECT COUNT(*) FROM person_roles \
             WHERE insight_tenant_id = ? AND person_id = ? AND role_id = ? \
               AND reason = 'bootstrap' AND valid_to IS NULL",
            [
                tenant.as_bytes().to_vec().into(),
                person.as_bytes().to_vec().into(),
                roles_repo::ADMIN_ROLE_ID.as_bytes().to_vec().into(),
            ],
        )
        .await?;
        assert_eq!(admins, 1, "bootstrap must be idempotent");
        Ok(())
    }
}
