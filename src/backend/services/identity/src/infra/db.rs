//! `MariaDB` connection and migration runner.

use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::time::Duration;

/// Connect to `MariaDB`.
///
/// # Errors
/// Returns an error if the database URL is invalid or the connection cannot be established.
pub async fn connect(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut opts = ConnectOptions::new(database_url);
    // Explicit timeouts so a misconfigured DSN or a half-open network
    // path fails fast at startup instead of hanging behind sqlx
    // defaults (acquire_timeout=30s).
    opts.max_connections(10)
        .min_connections(2)
        .connect_timeout(Duration::from_secs(10))
        .acquire_timeout(Duration::from_secs(10))
        .sqlx_logging(false);

    let db = Database::connect(opts).await?;
    tracing::info!("connected to database");
    Ok(db)
}

/// Run pending migrations.
///
/// # Errors
/// Returns an error if any migration fails to apply.
pub async fn run_migrations(db: &DatabaseConnection) -> anyhow::Result<()> {
    use sea_orm_migration::MigratorTrait;
    crate::migration::Migrator::up(db, None).await?;
    tracing::info!("migrations applied");
    Ok(())
}
