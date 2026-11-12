mod check_probe;
pub(crate) mod entities;
use std::time::Duration;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};

pub(crate) const MIGRATION_LOCK: &str = "connectors_migrations";
pub(crate) const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_mins(5);

pub(crate) async fn connect(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut options = ConnectOptions::new(database_url);
    options
        .max_connections(10)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(1))
        .map_sqlx_mysql_pool_opts(|options| {
            options.after_connect(|connection, _| {
                Box::pin(async move {
                    tokio::time::timeout(
                        Duration::from_secs(1),
                        sea_orm::sqlx::query("SET SESSION max_statement_time=1")
                            .execute(connection),
                    )
                    .await
                    .map_err(|_| sea_orm::sqlx::Error::PoolTimedOut)??;
                    Ok(())
                })
            })
        })
        .sqlx_logging(false);
    Ok(Database::connect(options).await?)
}

pub(crate) async fn assert_schema_compatible(db: &DatabaseConnection) -> anyhow::Result<()> {
    insight_migration::assert_compatible::<crate::migration::Migrator>(db).await?;
    check_probe::assert_required_checks(db).await
}
