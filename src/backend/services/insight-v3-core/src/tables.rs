use std::fmt;
use std::time::Duration;

use clickhouse::sql::Identifier;
use thiserror::Error;

pub(crate) const MAX_TABLE_NAME_CHARS: usize = 128;
const TABLE_CREATE_TIMEOUT_SECS: u64 = 35;
/// Catalogue reads answer from `system.tables` or twenty rows, so they get a
/// tighter bound than the DDL above. They run on the chat path.
const TABLE_READ_TIMEOUT_SECS: u64 = 10;
const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS ? (
    id UUID,
    table_name String,
    raw_data String,
    received_at DateTime64(3, 'UTC')
)
ENGINE = MergeTree
ORDER BY (table_name, received_at, id)";
/// The ingest tables, told apart from every other service's tables in the
/// same database by the ORDER BY that `CREATE_TABLE` above gives them.
const LIST_TABLES: &str = "SELECT name FROM system.tables
WHERE database = currentDatabase()
  AND sorting_key = 'table_name, received_at, id'
ORDER BY name";

#[derive(Debug, Clone)]
pub(crate) struct TableName(String);

impl TableName {
    pub(crate) fn parse(value: &str) -> Result<Self, TableError> {
        if value.is_empty() {
            return Err(TableError::EmptyName);
        }
        if value.chars().count() > MAX_TABLE_NAME_CHARS {
            return Err(TableError::NameTooLong);
        }

        let mut bytes = value.bytes();
        let Some(first) = bytes.next() else {
            return Err(TableError::EmptyName);
        };
        if !(first.is_ascii_alphabetic() || first == b'_')
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(TableError::InvalidName);
        }

        Ok(Self(value.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

pub(crate) struct TableStore {
    client: insight_clickhouse::Client,
    create_timeout: Duration,
    read_timeout: Duration,
}

impl TableStore {
    pub(crate) fn new(client: insight_clickhouse::Client) -> Self {
        Self {
            client,
            create_timeout: Duration::from_secs(TABLE_CREATE_TIMEOUT_SECS),
            read_timeout: Duration::from_secs(TABLE_READ_TIMEOUT_SECS),
        }
    }

    #[cfg(test)]
    fn with_read_timeout(client: insight_clickhouse::Client, read_timeout: Duration) -> Self {
        Self {
            client,
            create_timeout: Duration::from_secs(TABLE_CREATE_TIMEOUT_SECS),
            read_timeout,
        }
    }

    pub(crate) async fn create(&self, table: &TableName) -> Result<(), TableStoreError> {
        let query = self
            .client
            .inner()
            .query(CREATE_TABLE)
            .bind(Identifier(table.as_str()));

        tokio::time::timeout(self.create_timeout, query.execute())
            .await
            .map_err(|_| TableStoreError::Timeout)??;

        Ok(())
    }

    /// Every table data has been ingested into.
    pub(crate) async fn list(&self) -> Result<Vec<String>, TableStoreError> {
        let names = self.client.inner().query(LIST_TABLES).fetch_all::<String>();

        Ok(tokio::time::timeout(self.read_timeout, names)
            .await
            .map_err(|_| TableStoreError::Timeout)??)
    }

    /// Field names and inferred types, read from the most recent rows.
    pub(crate) async fn sample_fields(
        &self,
        table: &TableName,
    ) -> Result<Vec<(String, &'static str)>, TableStoreError> {
        let sql = format!(
            "SELECT raw_data FROM `{}` ORDER BY received_at DESC LIMIT 20",
            table.as_str()
        );

        let rows = self.client.inner().query(&sql).fetch_all::<String>();
        let payloads = tokio::time::timeout(self.read_timeout, rows)
            .await
            .map_err(|_| TableStoreError::Timeout)??;
        let mut fields: Vec<(String, &'static str)> = Vec::new();

        for payload in payloads {
            let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&payload) else {
                continue;
            };

            for (key, value) in map {
                if fields.iter().any(|(name, _)| name == &key) {
                    continue;
                }

                let kind = match value {
                    serde_json::Value::Number(number) if number.is_i64() => "int",
                    serde_json::Value::Number(_) => "float",
                    _ => "string",
                };

                fields.push((key, kind));
            }
        }

        Ok(fields)
    }
}

impl fmt::Debug for TableStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TableStore")
            .field("create_timeout", &self.create_timeout)
            .field("read_timeout", &self.read_timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Error)]
pub(crate) enum TableError {
    #[error("table must not be blank")]
    EmptyName,
    #[error("table must be at most {MAX_TABLE_NAME_CHARS} characters")]
    NameTooLong,
    #[error(
        "table must start with an ASCII letter or underscore and contain only ASCII letters, digits, or underscores"
    )]
    InvalidName,
}

/// Shared by creation, listing and sampling, so neither message names one of
/// the three — the route that only ever creates says so in its own reply.
#[derive(Debug, Error)]
pub(crate) enum TableStoreError {
    #[error("the warehouse did not answer in time")]
    Timeout,
    #[error("the warehouse refused the request")]
    ClickHouse(#[from] clickhouse::error::Error),
}

#[cfg(test)]
mod tests {
    use clickhouse::test::{Mock, handlers};

    use super::*;

    #[test]
    fn physical_table_names_use_portable_identifiers() {
        assert!(TableName::parse("events_2026").is_ok());

        for value in ["", " events", "events.daily", "9events", "naïve"] {
            assert!(
                TableName::parse(value).is_err(),
                "should reject table name: {value:?}"
            );
        }
    }

    #[tokio::test]
    async fn table_creation_uses_the_fixed_raw_data_schema() {
        let mock = Mock::new();
        let recording = mock.add(handlers::record_ddl());
        let client =
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));
        let store = TableStore::new(client);
        let table = TableName::parse("events_2026").unwrap_or_else(|error| panic!("name: {error}"));

        store
            .create(&table)
            .await
            .unwrap_or_else(|error| panic!("table should be created: {error}"));
        let query = recording.query().await;

        assert!(query.contains("CREATE TABLE IF NOT EXISTS `events_2026`"));
        assert!(query.contains("id UUID"));
        assert!(query.contains("table_name String"));
        assert!(query.contains("raw_data String"));
        assert!(query.contains("received_at DateTime64(3, 'UTC')"));
        assert!(query.contains("ORDER BY (table_name, received_at, id)"));
    }

    #[tokio::test]
    async fn a_hung_listing_is_time_bounded() {
        let (address, server) = hanging_warehouse().await;
        let store = TableStore::with_read_timeout(
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                format!("http://{address}"),
                "insight",
            )),
            Duration::from_millis(25),
        );

        let result = tokio::time::timeout(Duration::from_secs(1), store.list()).await;
        server.abort();

        assert!(matches!(result, Ok(Err(TableStoreError::Timeout))));
    }

    #[tokio::test]
    async fn a_hung_field_sample_is_time_bounded() {
        let (address, server) = hanging_warehouse().await;
        let store = TableStore::with_read_timeout(
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                format!("http://{address}"),
                "insight",
            )),
            Duration::from_millis(25),
        );
        let table = TableName::parse("events_2026").unwrap_or_else(|error| panic!("name: {error}"));

        let result =
            tokio::time::timeout(Duration::from_secs(1), store.sample_fields(&table)).await;
        server.abort();

        assert!(matches!(result, Ok(Err(TableStoreError::Timeout))));
    }

    #[test]
    fn no_failure_of_a_read_claims_a_creation_failed() {
        assert!(!TableStoreError::Timeout.to_string().contains("creation"));
        assert!(
            !TableStoreError::ClickHouse(clickhouse::error::Error::Custom("x".to_owned()))
                .to_string()
                .contains("creation")
        );
    }

    /// Accepts the connection and then answers nothing at all.
    async fn hanging_warehouse() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|error| panic!("test listener must bind: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("test listener must have an address: {error}"));
        let server = tokio::spawn(async move {
            let _connection = listener
                .accept()
                .await
                .unwrap_or_else(|error| panic!("test server must accept: {error}"));
            futures::future::pending::<()>().await;
        });

        (address, server)
    }

    #[test]
    fn listing_selects_only_tables_shaped_like_an_ingest_table() {
        // The database holds tables from every other service too, so the
        // ingest schema's own ORDER BY is what separates ours from theirs.
        assert!(LIST_TABLES.contains("system.tables"));
        assert!(LIST_TABLES.contains("database = currentDatabase()"));
        assert!(LIST_TABLES.contains("table_name, received_at, id"));
    }

    #[tokio::test]
    async fn listing_returns_the_table_names() {
        let mock = Mock::new();
        mock.add(handlers::provide(vec![
            "events".to_owned(),
            "raw_data".to_owned(),
        ]));
        let client =
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));
        let store = TableStore::new(client);

        let names = store
            .list()
            .await
            .unwrap_or_else(|error| panic!("tables should list: {error}"));

        assert_eq!(names, vec!["events".to_owned(), "raw_data".to_owned()]);
    }

    #[tokio::test]
    async fn overlapping_rows_yield_one_entry_per_field_typed_from_its_value() {
        let mock = Mock::new();
        mock.add(handlers::provide(vec![
            r#"{"day":"2026-09-01","lines":59}"#.to_owned(),
            r#"{"day":"2026-09-02","author":"nda"}"#.to_owned(),
        ]));
        let client =
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));
        let store = TableStore::new(client);
        let table = TableName::parse("events_2026").unwrap_or_else(|error| panic!("name: {error}"));

        let fields = store
            .sample_fields(&table)
            .await
            .unwrap_or_else(|error| panic!("fields should be sampled: {error}"));

        assert_eq!(
            fields,
            vec![
                ("day".to_owned(), "string"),
                ("lines".to_owned(), "int"),
                ("author".to_owned(), "string"),
            ]
        );
    }
}
