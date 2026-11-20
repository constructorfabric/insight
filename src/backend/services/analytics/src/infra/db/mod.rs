pub mod check_probe;
pub mod entities;
#[cfg(test)]
mod live_tests;

use std::time::Duration;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};

pub async fn connect(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut opts = ConnectOptions::new(database_url);
    opts.max_connections(10)
        .min_connections(2)
        .sqlx_logging(false);

    let db = Database::connect(opts).await?;
    tracing::info!("connected to database");
    Ok(db)
}

pub const MIGRATION_LOCK: &str = "analytics_migrations";

pub const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_mins(5);

pub async fn assert_schema_compatible(db: &DatabaseConnection) -> anyhow::Result<()> {
    Ok(insight_migration::assert_compatible::<crate::migration::Migrator>(db).await?)
}
