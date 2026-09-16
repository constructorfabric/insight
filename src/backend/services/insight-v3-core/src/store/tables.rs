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
mod tests;
