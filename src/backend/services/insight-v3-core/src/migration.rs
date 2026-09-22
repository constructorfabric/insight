use clickhouse::sql::Identifier;
use thiserror::Error;

/// The database every dataset's table lives in, and nothing else does.
///
/// A dataset's table is created when the dataset is, so the database has to
/// stand before the first declaration - and nothing else this service does
/// would make it.
const CREATE_DATASETS_DATABASE: &str = "CREATE DATABASE IF NOT EXISTS ?";

pub(crate) async fn migrate(
    client: &insight_clickhouse::Client,
    datasets_database: &str,
) -> Result<(), MigrationError> {
    client
        .inner()
        .query(CREATE_DATASETS_DATABASE)
        .bind(Identifier(datasets_database))
        .execute()
        .await?;

    Ok(())
}

#[derive(Debug, Error)]
#[error("failed to create the datasets database")]
pub(crate) struct MigrationError(#[from] clickhouse::error::Error);

#[cfg(test)]
mod tests {
    use clickhouse::test::{Mock, handlers};

    use super::*;

    #[tokio::test]
    async fn the_migration_makes_the_datasets_database_and_nothing_in_the_warehouse() {
        let mock = Mock::new();
        let recording = mock.add(handlers::record_ddl());
        let client =
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));

        migrate(&client, "insight_datasets")
            .await
            .unwrap_or_else(|error| panic!("migration must succeed: {error}"));
        let ddl = recording.query().await;

        assert_eq!(ddl, "CREATE DATABASE IF NOT EXISTS `insight_datasets`");
    }
}
