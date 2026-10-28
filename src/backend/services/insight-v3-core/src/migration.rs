use thiserror::Error;

/// The datasets database is `create-databases.sh`'s to make; a migration
/// only refuses to proceed against one that is missing or whose engine
/// contradicts the configured topology.
pub(crate) async fn migrate(
    client: &insight_clickhouse::Client,
    datasets_database: &str,
) -> Result<(), MigrationError> {
    client.assert_database_topology(datasets_database).await?;

    Ok(())
}

#[derive(Debug, Error)]
#[error("the datasets database is not ready: {0}")]
pub(crate) struct MigrationError(#[from] insight_clickhouse::Error);

#[cfg(test)]
mod tests {
    use clickhouse::test::{Mock, handlers};

    use super::*;

    fn client(mock: &Mock, topology: insight_clickhouse::Topology) -> insight_clickhouse::Client {
        insight_clickhouse::Client::new(
            insight_clickhouse::Config::new(mock.url(), "insight").with_topology(topology),
        )
    }

    /// One answer from `system.databases` is all the migration consumes: the
    /// mock holds no handler for a `CREATE`, so one would fail the call.
    #[tokio::test]
    async fn the_migration_creates_nothing_and_passes_a_database_of_the_right_engine() {
        let mock = Mock::new();
        mock.add(handlers::provide(vec![String::from("Atomic")]));

        migrate(
            &client(&mock, insight_clickhouse::Topology::Standalone),
            "insight_datasets",
        )
        .await
        .unwrap_or_else(|error| panic!("migration must succeed: {error}"));
    }

    #[tokio::test]
    async fn a_missing_datasets_database_is_reported_not_created() {
        let mock = Mock::new();
        mock.add(handlers::provide(Vec::<String>::new()));

        let refused = migrate(
            &client(&mock, insight_clickhouse::Topology::Standalone),
            "insight_datasets",
        )
        .await;

        let Err(error) = refused else {
            panic!("a missing database is refused");
        };
        assert!(error.to_string().contains("create-databases.sh"), "{error}");
    }

    #[tokio::test]
    async fn a_plain_database_is_refused_under_a_replicated_topology() {
        let mock = Mock::new();
        mock.add(handlers::provide(vec![String::from("Atomic")]));

        let refused = migrate(
            &client(&mock, insight_clickhouse::Topology::Replicated),
            "insight_datasets",
        )
        .await;

        let Err(error) = refused else {
            panic!("a plain database is refused");
        };
        assert!(
            error.to_string().contains("clickhouse_cluster_mode is on"),
            "{error}"
        );
    }
}
