//! `SeaORM` cannot express [`assert_compatible`]:
//! every `MigratorTrait` status accessor — including the 2.0 `*_read_only`
//! variants — treats `applied - embedded` as a fatal "migration file is
//! missing" error, which is exactly the rollback case this gate exists to
//! allow. The ledger is therefore read directly through the public
//! [`sea_orm_migration::seaql_migrations`] entity and compared here.

pub mod squash;

use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, DbErr, EntityTrait,
    Statement, Value,
};
use sea_orm_migration::{MigratorTrait, SchemaManager, seaql_migrations};

const MAX_REPORTED_MISSING: usize = 5;

/// `sea-orm` exposes `idle_timeout`/`max_lifetime` as `Duration` and forwards
/// them to `sqlx` only when set, so leaving them unset means `sqlx`'s own
/// defaults apply — a 10-minute idle timeout and a 30-minute maximum lifetime.
/// Either would reap the connection holding the advisory lock partway through
/// a long migration and hand the next statement a fresh session with no lock.
/// `None` is not reachable through `ConnectOptions`, so the ceiling is pushed
/// far past any plausible migration instead.
const SESSION_LIFETIME: Duration = Duration::from_hours(24);

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("connection URL selects no database — the migration ledger cannot be located")]
    NoDatabaseSelected,

    #[error("unsupported custom migration table name `{0}`")]
    UnsupportedTableName(String),

    #[error(
        "schema is behind this build: {missing} of {embedded} embedded migrations are not \
         recorded applied: {shown}. Run this service's `migrate` entrypoint against the \
         database before starting the server."
    )]
    SchemaBehind {
        missing: usize,
        embedded: usize,
        shown: String,
    },

    #[error(
        "could not acquire the `{lock_name}` advisory lock within {timeout_secs}s — is \
         another migration run stuck?"
    )]
    LockTimeout {
        lock_name: String,
        timeout_secs: u64,
    },

    #[error(
        "the `{lock_name}` advisory lock was no longer held by this session when the \
         migration finished — the connection was replaced mid-run, so the migration cannot \
         be assumed to have run serialized. Inspect the schema before retrying."
    )]
    LockLost { lock_name: String },

    #[error("{0} returned no row")]
    ProbeEmpty(&'static str),

    #[error(transparent)]
    Db(#[from] DbErr),

    #[error(transparent)]
    Body(anyhow::Error),
}

/// Returns an empty set when the ledger table does not exist, which is the
/// fresh-database case: no migration has been applied yet. Never issues DDL —
/// a server boot must not create tables, so the `install`-first
/// `MigratorTrait` accessors are deliberately avoided.
///
/// # Errors
///
/// Returns an error if the connection selects no default database, the
/// ledger probe fails, or the ledger read fails.
pub async fn applied_versions<M>(
    db: &DatabaseConnection,
) -> Result<BTreeSet<String>, MigrationError>
where
    M: MigratorTrait,
{
    assert_default_database_selected(db).await?;

    // INVARIANT: the entity below is hardwired to the default table name, so
    // a migrator that overrides `migration_table_name` would silently read
    // the wrong table.
    let table = M::migration_table_name().to_string();
    if table != "seaql_migrations" {
        return Err(MigrationError::UnsupportedTableName(table));
    }

    if !SchemaManager::new(db).has_table(&table).await? {
        return Ok(BTreeSet::new());
    }

    Ok(seaql_migrations::Entity::find()
        .all(db)
        .await?
        .into_iter()
        .map(|model| model.version)
        .collect())
}

/// Refuse to serve unless the live schema carries every migration this build
/// embeds.
///
/// Migrations recorded in the ledger that this build does not embed are
/// tolerated and logged: that is a deployed application rollback, and it is
/// supported. The reverse — an embedded migration with no ledger row — means
/// the schema is behind the binary, and serving would hit missing columns at
/// query time.
///
/// # Errors
///
/// Returns an error if the ledger cannot be read, or if any embedded migration
/// is not recorded applied.
pub async fn assert_compatible<M>(db: &DatabaseConnection) -> Result<(), MigrationError>
where
    M: MigratorTrait,
{
    let embedded: BTreeSet<String> = M::migrations()
        .iter()
        .map(|migration| migration.name().to_owned())
        .collect();
    let applied = applied_versions::<M>(db).await?;

    let missing: Vec<&str> = embedded.difference(&applied).map(String::as_str).collect();
    if !missing.is_empty() {
        let shown = missing
            .iter()
            .take(MAX_REPORTED_MISSING)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        let elided = missing.len().saturating_sub(MAX_REPORTED_MISSING);
        let suffix = if elided == 0 {
            String::new()
        } else {
            format!(" (+{elided} more)")
        };
        return Err(MigrationError::SchemaBehind {
            missing: missing.len(),
            embedded: embedded.len(),
            shown: format!("{shown}{suffix}"),
        });
    }

    let newer = applied.difference(&embedded).count();
    if newer == 0 {
        tracing::info!(
            embedded = embedded.len(),
            "schema matches this build's migration set"
        );
    } else {
        tracing::info!(
            embedded = embedded.len(),
            applied = applied.len(),
            newer,
            "schema carries migrations newer than this build; tolerated under \
             the forward-only contract (this build is running as a rollback)"
        );
    }
    Ok(())
}

/// Run `body` on a pinned single-connection session holding the `lock_name`
/// advisory lock.
///
/// `MariaDB` DDL is not transactional, so two migrators racing can double-apply
/// a pending script. `GET_LOCK` serializes them, but only per session — hence
/// the pool this function builds is capped at one connection and its reaping
/// timeouts are pushed out past any plausible migration. The lock is released
/// on both the success and failure paths, and also dies with the session.
///
/// After `body` completes, the session is re-checked for lock ownership. A
/// connection that was replaced underneath the migration would have silently
/// dropped the lock, so that case fails loudly rather than reporting a
/// migration that ran unserialized.
///
/// # Errors
///
/// Returns an error if the connection cannot be established, the lock cannot be
/// acquired within `lock_timeout`, `body` fails, or the session no longer held
/// the lock when `body` finished.
pub async fn with_migration_session<T, F, Fut>(
    database_url: &str,
    lock_name: &str,
    lock_timeout: Duration,
    body: F,
) -> Result<T, MigrationError>
where
    F: FnOnce(DatabaseConnection) -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    let db = connect_pinned(database_url).await?;
    acquire_lock(&db, lock_name, lock_timeout).await?;

    let outcome = body(db.clone()).await;
    let still_held = holds_lock(&db, lock_name).await;

    let _ = db
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            "SELECT RELEASE_LOCK(?)",
            [Value::from(lock_name)],
        ))
        .await;

    let value = outcome.map_err(MigrationError::Body)?;
    if !still_held? {
        return Err(MigrationError::LockLost {
            lock_name: lock_name.to_owned(),
        });
    }
    Ok(value)
}

async fn connect_pinned(database_url: &str) -> Result<DatabaseConnection, MigrationError> {
    let mut opts = ConnectOptions::new(database_url);
    opts.max_connections(1)
        .min_connections(1)
        .idle_timeout(SESSION_LIFETIME)
        .max_lifetime(SESSION_LIFETIME)
        .test_before_acquire(false)
        .sqlx_logging(false);

    let db = Database::connect(opts).await?;
    tracing::info!("connected to MariaDB (pinned migration session)");
    Ok(db)
}

async fn acquire_lock(
    db: &DatabaseConnection,
    lock_name: &str,
    lock_timeout: Duration,
) -> Result<(), MigrationError> {
    let timeout_secs = lock_timeout.as_secs();
    // CAST so the column type is deterministic. The lock builtins are typed by
    // the server per expression, so an uncast result can arrive as INT, BIGINT,
    // or BIGINT UNSIGNED and fail decoding on some servers but not others.
    let acquired: Option<i64> = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            "SELECT CAST(GET_LOCK(?, ?) AS SIGNED)",
            [Value::from(lock_name), Value::from(timeout_secs)],
        ))
        .await?
        .map(|row| row.try_get_by_index::<Option<i64>>(0))
        .transpose()?
        .flatten();

    if acquired != Some(1) {
        return Err(MigrationError::LockTimeout {
            lock_name: lock_name.to_owned(),
            timeout_secs,
        });
    }
    Ok(())
}

async fn holds_lock(db: &DatabaseConnection, lock_name: &str) -> Result<bool, MigrationError> {
    // Compared server-side and CAST for the same reason as `acquire_lock`.
    // `IS_USED_LOCK` yields NULL when nobody holds the lock, so the comparison
    // is NULL too and decodes to `None` — correctly, "not held by us".
    let held: Option<i64> = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            "SELECT CAST(IS_USED_LOCK(?) = CONNECTION_ID() AS SIGNED)",
            [Value::from(lock_name)],
        ))
        .await?
        .ok_or(MigrationError::ProbeEmpty("lock ownership probe"))?
        .try_get_by_index(0)?;

    Ok(held == Some(1))
}

async fn assert_default_database_selected(db: &DatabaseConnection) -> Result<(), MigrationError> {
    let selected: Option<String> = db
        .query_one_raw(Statement::from_string(
            DbBackend::MySql,
            "SELECT DATABASE()",
        ))
        .await?
        .ok_or(MigrationError::ProbeEmpty("DATABASE() probe"))?
        .try_get_by_index(0)?;

    if selected.is_none() {
        return Err(MigrationError::NoDatabaseSelected);
    }
    Ok(())
}
