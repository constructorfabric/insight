use thiserror::Error;

const CREATE_RAW_DATA_TABLE: &str = "CREATE TABLE IF NOT EXISTS raw_data (
    id UUID,
    table_name String,
    raw_data String,
    received_at DateTime64(3, 'UTC')
)
ENGINE = MergeTree
ORDER BY (table_name, received_at, id)";

/// The ingest landing table. The definitions used to be created here too;
/// they live in `MariaDB` now — see [`crate::definitions`].
pub(crate) async fn migrate(client: &insight_clickhouse::Client) -> Result<(), MigrationError> {
    client
        .inner()
        .query(CREATE_RAW_DATA_TABLE)
        .execute()
        .await?;

    Ok(())
}

#[derive(Debug, Error)]
#[error("failed to migrate the raw_data table")]
pub(crate) struct MigrationError(#[from] clickhouse::error::Error);

#[cfg(test)]
mod tests {
    use clickhouse::test::{Mock, handlers};

    use super::*;

    #[tokio::test]
    async fn migration_creates_the_raw_data_table() {
        let mock = Mock::new();
        let recording = mock.add(handlers::record_ddl());
        let client =
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));

        migrate(&client)
            .await
            .unwrap_or_else(|error| panic!("migration must succeed: {error}"));
        let ddl = recording.query().await;

        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS raw_data"));
        assert!(ddl.contains("table_name String"));
        assert!(ddl.contains("raw_data String"));
        assert!(ddl.contains("received_at DateTime64(3, 'UTC')"));
        assert!(ddl.contains("ORDER BY (table_name, received_at, id)"));
    }
}
