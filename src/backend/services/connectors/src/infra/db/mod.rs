use std::time::Duration;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};

pub(crate) const MIGRATION_LOCK: &str = "connectors_migrations";
pub(crate) const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_mins(5);

pub(crate) async fn connect(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut options = ConnectOptions::new(database_url);
    options
        .max_connections(10)
        .min_connections(2)
        .sqlx_logging(false);
    Ok(Database::connect(options).await?)
}

pub(crate) async fn assert_schema_compatible(db: &DatabaseConnection) -> anyhow::Result<()> {
    Ok(insight_migration::assert_compatible::<crate::migration::Migrator>(db).await?)
}
