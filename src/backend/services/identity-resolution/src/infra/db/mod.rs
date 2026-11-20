pub mod bootstrap;
pub mod entities;
pub mod ops_repo;
pub mod people_listing;
pub mod people_repo;
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
mod visible_set_sql;

#[cfg(test)]
mod binding_reads_live_tests;
#[cfg(test)]
mod email_map_live_tests;
#[cfg(test)]
mod person_listing_live_tests;
#[cfg(test)]
mod profile_resolve_live_tests;
#[cfg(test)]
mod roster_email_live_tests;
#[cfg(test)]
mod roster_live_tests;
#[cfg(test)]
pub(crate) mod test_fixture;
#[cfg(test)]
mod visible_set_live_tests;

use std::time::Duration;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};

pub async fn connect(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut opts = ConnectOptions::new(database_url);
    opts.max_connections(10)
        .min_connections(2)
        .sqlx_logging(false);

    let db = Database::connect(opts).await?;
    tracing::info!("connected to MariaDB");
    Ok(db)
}

/// `GET_LOCK` binds to the session, so the lock, the work it guards, and the
/// release must all execute on the same connection.
pub async fn connect_single(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut opts = ConnectOptions::new(database_url);
    opts.max_connections(1)
        .min_connections(1)
        .sqlx_logging(false);

    let db = Database::connect(opts).await?;
    tracing::info!("connected to MariaDB (single-connection session)");
    Ok(db)
}

pub const MIGRATION_LOCK: &str = "identity_resolution_migrations";
pub const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_mins(5);

pub async fn assert_schema_compatible(db: &DatabaseConnection) -> anyhow::Result<()> {
    Ok(insight_migration::assert_compatible::<crate::migration::Migrator>(db).await?)
}

const SEED_LOCK_PREFIX: &str = "persons-seed:";

/// Dropping the owned connection closes the session, and MariaDB releases a
/// `GET_LOCK` lock server-side when its session ends.
pub struct SeedLockGuard {
    conn: DatabaseConnection,
    tenant_id: uuid::Uuid,
}

impl SeedLockGuard {
    pub async fn acquire(
        database_url: &str,
        tenant_id: uuid::Uuid,
        wait_secs: u32,
    ) -> anyhow::Result<Option<Self>> {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        let conn = connect_single(database_url).await?;
        // SAFETY: CAST so the column type is deterministic — the lock builtins
        // are typed per expression by the server, and an uncast result can
        // arrive as INT, BIGINT, or BIGINT UNSIGNED and fail decoding.
        let acquired: Option<i64> = conn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                "SELECT CAST(GET_LOCK(?, ?) AS SIGNED)",
                [
                    format!("{SEED_LOCK_PREFIX}{tenant_id}").into(),
                    wait_secs.into(),
                ],
            ))
            .await?
            .map(|r| r.try_get_by_index::<Option<i64>>(0))
            .transpose()?
            .flatten();
        if acquired == Some(1) {
            Ok(Some(Self { conn, tenant_id }))
        } else {
            Ok(None)
        }
    }

    pub async fn release(self) {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        let _ = self
            .conn
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                "SELECT RELEASE_LOCK(?)",
                [format!("{SEED_LOCK_PREFIX}{}", self.tenant_id).into()],
            ))
            .await;
    }
}

const SYNC_LOCK: &str = "persons-sync";

pub struct SyncLockGuard {
    conn: DatabaseConnection,
}

impl SyncLockGuard {
    pub async fn acquire(database_url: &str, wait_secs: u32) -> anyhow::Result<Option<Self>> {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        let conn = connect_single(database_url).await?;
        // SAFETY: CAST for the same deterministic-decode reason as
        // `SeedLockGuard::acquire`.
        let acquired: Option<i64> = conn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                "SELECT CAST(GET_LOCK(?, ?) AS SIGNED)",
                [SYNC_LOCK.into(), wait_secs.into()],
            ))
            .await?
            .map(|r| r.try_get_by_index::<Option<i64>>(0))
            .transpose()?
            .flatten();
        if acquired == Some(1) {
            Ok(Some(Self { conn }))
        } else {
            Ok(None)
        }
    }

    pub async fn release(self) {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        let _ = self
            .conn
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                "SELECT RELEASE_LOCK(?)",
                [SYNC_LOCK.into()],
            ))
            .await;
    }
}

pub async fn run_migrations(
    database_url: &str,
    config: &crate::config::GearConfig,
) -> anyhow::Result<()> {
    insight_migration::with_migration_session(
        database_url,
        MIGRATION_LOCK,
        MIGRATION_LOCK_TIMEOUT,
        |db| async move {
            use sea_orm_migration::MigratorTrait;
            crate::migration::Migrator::up(&db, None).await?;
            bootstrap::bootstrap_admin(&db, config).await
        },
    )
    .await?;
    tracing::info!("migrations + bootstrap applied");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use sea_orm::{ConnectionTrait, DbBackend, Statement};
    use sea_orm_migration::MigratorTrait;

    use super::*;
    use crate::config::GearConfig;

    static LEDGER_MUTEX: LazyLock<tokio::sync::Mutex<()>> =
        LazyLock::new(|| tokio::sync::Mutex::new(()));

    const BASELINE: &str = "m20261030_000001_baseline";
    const PARTIAL: &str = "m20260724_000012_org_chart_nullable_parent";

    async fn forget(db: &DatabaseConnection, versions: &[&str]) -> anyhow::Result<()> {
        for version in versions {
            db.execute_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                "DELETE FROM seaql_migrations WHERE version = ?",
                [(*version).into()],
            ))
            .await?;
        }
        Ok(())
    }

    async fn count(db: &DatabaseConnection, sql: &str) -> anyhow::Result<i64> {
        count_with(db, sql, []).await
    }

    async fn count_with(
        db: &DatabaseConnection,
        sql: &str,
        values: impl IntoIterator<Item = sea_orm::Value>,
    ) -> anyhow::Result<i64> {
        let row = db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                sql,
                values,
            ))
            .await?
            .ok_or_else(|| anyhow::anyhow!("count query returned no row"))?;
        Ok(row.try_get_by_index::<i64>(0)?)
    }

    #[tokio::test]
    async fn migrations_and_bootstrap_are_idempotent_against_live_mariadb() -> anyhow::Result<()> {
        let Ok(url) = std::env::var("INTEGRATION_TESTS_MARIADB_URL") else {
            eprintln!("skip: set INTEGRATION_TESTS_MARIADB_URL to run");
            return Ok(());
        };
        let _serialized = LEDGER_MUTEX.lock().await;
        let db = connect(&url).await?;
        let cfg = GearConfig {
            tenant_default_id: "3e1d5a65-434c-95b4-8c1b-eb8f53a39bab".to_owned(),
            bootstrap_admin_person_id: "019e27bc-dec0-7626-81a9-c5524662a6a9".to_owned(),
            ..GearConfig::default()
        };

        run_migrations(&url, &cfg).await?;
        run_migrations(&url, &cfg).await?;

        assert_schema_compatible(&db).await?;
        let applied = count(&db, "SELECT COUNT(*) FROM seaql_migrations").await?;
        let embedded = i64::try_from(crate::migration::Migrator::migrations().len())?;
        assert!(
            applied >= embedded,
            "ledger must hold at least the embedded set (applied {applied} < embedded {embedded})"
        );

        forget(&db, &[BASELINE]).await?;
        assert!(
            assert_schema_compatible(&db).await.is_err(),
            "a missing embedded migration must fail the boot gate"
        );
        run_migrations(&url, &cfg).await?;
        assert_schema_compatible(&db).await?;

        forget(&db, &[BASELINE, PARTIAL]).await?;
        let refused = run_migrations(&url, &cfg).await;
        db.execute_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            "INSERT INTO seaql_migrations (version, applied_at) VALUES (?, 0)",
            [PARTIAL.into()],
        ))
        .await?;
        let Err(error) = refused else {
            panic!("a partial squashed history must be refused");
        };
        assert!(
            format!("{error:#}").contains(&format!("`{PARTIAL}`")),
            "the refusal must name the first missing migration: {error:#}"
        );
        run_migrations(&url, &cfg).await?;
        assert_schema_compatible(&db).await?;

        let mut whole_history = vec![BASELINE];
        whole_history.extend_from_slice(crate::migration::SQUASHED);
        forget(&db, &whole_history).await?;
        run_migrations(&url, &cfg).await?;
        assert_schema_compatible(&db).await?;
        // INVARIANT: keep the schema filter — `information_schema` spans the
        // server, so without it a shared instance counts every identity schema.
        let checks = count(
            &db,
            "SELECT COUNT(*) FROM information_schema.CHECK_CONSTRAINTS \
             WHERE CONSTRAINT_NAME = 'chk_no_self_loop' \
               AND CONSTRAINT_SCHEMA = DATABASE()",
        )
        .await?;
        assert_eq!(checks, 1, "the baseline re-run must keep the constraint");
        let roles = count(&db, "SELECT COUNT(*) FROM roles").await?;
        assert!(roles >= 2, "the baseline re-run must keep the seeded roles");

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

    #[tokio::test]
    async fn newer_ledger_rows_do_not_block_boot() -> anyhow::Result<()> {
        const FUTURE: &str = "m99999999_000001_from_a_newer_release";

        let Ok(url) = std::env::var("INTEGRATION_TESTS_MARIADB_URL") else {
            eprintln!("skip: set INTEGRATION_TESTS_MARIADB_URL to run");
            return Ok(());
        };
        let _serialized = LEDGER_MUTEX.lock().await;
        let db = connect(&url).await?;
        let cfg = GearConfig {
            tenant_default_id: "3e1d5a65-434c-95b4-8c1b-eb8f53a39bab".to_owned(),
            bootstrap_admin_person_id: "019e27bc-dec0-7626-81a9-c5524662a6a9".to_owned(),
            ..GearConfig::default()
        };
        run_migrations(&url, &cfg).await?;

        db.execute_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            "INSERT INTO seaql_migrations (version, applied_at) VALUES (?, 0)",
            [FUTURE.into()],
        ))
        .await?;

        let tolerated = assert_schema_compatible(&db).await;

        db.execute_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            "DELETE FROM seaql_migrations WHERE version = ?",
            [FUTURE.into()],
        ))
        .await?;

        tolerated?;
        Ok(())
    }
}
