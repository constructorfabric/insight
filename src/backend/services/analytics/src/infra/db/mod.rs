//! `MariaDB` connection and repository.

pub mod check_probe;
pub mod entities;
#[cfg(test)]
mod live_tests;

use std::time::Duration;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};

/// Connect to `MariaDB`.
///
/// # Errors
///
/// Returns error if the connection fails.
pub async fn connect(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut opts = ConnectOptions::new(database_url);
    opts.max_connections(10)
        .min_connections(2)
        .sqlx_logging(false);

    let db = Database::connect(opts).await?;
    tracing::info!("connected to database");
    Ok(db)
}

/// Name of the cross-process advisory lock serializing schema migration runs.
pub const MIGRATION_LOCK: &str = "analytics_migrations";

/// How long a second migrator waits for the lock before giving up.
pub const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_mins(5);

/// Refuse to serve unless the live schema carries every migration this build
/// embeds.
///
/// The server never applies migrations — the `migrate` entrypoint does. Rows
/// applied by a newer release are tolerated so an application image can be
/// rolled back over a forward-only schema.
///
/// # Errors
///
/// Returns error if the ledger cannot be read or the schema is behind this
/// build.
pub async fn assert_schema_compatible(db: &DatabaseConnection) -> anyhow::Result<()> {
    Ok(insight_migration::assert_compatible::<crate::migration::Migrator>(db).await?)
}
