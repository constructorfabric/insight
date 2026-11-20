use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use clickhouse::sql::Identifier;
use insight_clickhouse::{Ddl, MissingRelation, TableSpec, Unfinished};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const COLUMNS: [(&str, &str); 4] = [
    ("id", "UUID"),
    ("table_name", "String"),
    ("raw_data", "String"),
    ("received_at", "DateTime64(3, 'UTC')"),
];
const SORT_KEY: [&str; 3] = ["table_name", "received_at", "id"];
const INGEST_SORTING_KEY: &str = "table_name, received_at, id";
const READ_PAGE: &str = "SELECT id, received_at, raw_data FROM ?
ORDER BY {order}, received_at DESC, id DESC
LIMIT ? OFFSET ?";
const COUNT_ROWS: &str = "SELECT count() AS total FROM ?";
const READ_SHAPE: &str = "SELECT sorting_key FROM system.tables
WHERE database = currentDatabase() AND name = ?";
const DDL_TIMEOUT: Duration =
    insight_clickhouse::DDL_TASK_TIMEOUT.saturating_add(Duration::from_secs(5));
const READ_TIMEOUT_SECS: u64 = 10;
const INSERT_SEND_TIMEOUT_SECS: u64 = 10;
const INSERT_END_TIMEOUT_SECS: u64 = 30;
const INSERT_TOTAL_TIMEOUT_SECS: u64 = 35;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Absent,
    Ingest,
    Foreign,
}

pub(crate) struct DatasetTables {
    client: insight_clickhouse::Client,
    ddl_timeout: Duration,
    read_timeout: Duration,
    insert: InsertTimeouts,
}

#[derive(Debug, Clone, Copy)]
struct InsertTimeouts {
    send: Duration,
    end: Duration,
    total: Duration,
}

impl DatasetTables {
    pub(crate) fn new(client: insight_clickhouse::Client) -> Self {
        Self {
            client,
            ddl_timeout: DDL_TIMEOUT,
            read_timeout: Duration::from_secs(READ_TIMEOUT_SECS),
            insert: InsertTimeouts {
                send: Duration::from_secs(INSERT_SEND_TIMEOUT_SECS),
                end: Duration::from_secs(INSERT_END_TIMEOUT_SECS),
                total: Duration::from_secs(INSERT_TOTAL_TIMEOUT_SECS),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn with_ddl_timeout(mut self, bound: Duration) -> Self {
        self.ddl_timeout = bound;
        self
    }

    pub(crate) async fn insert(
        &self,
        table: &str,
        dataset: &str,
        raw_data: &str,
    ) -> Result<(), DatasetTableError> {
        let row = RecordRow {
            id: Uuid::now_v7(),
            table_name: dataset.to_owned(),
            raw_data: raw_data.to_owned(),
            received_at: Utc::now(),
        };

        tokio::time::timeout(self.insert.total, self.write(table, &row))
            .await
            .map_err(|_| DatasetTableError::Timeout)?
    }

    async fn write(&self, table: &str, row: &RecordRow) -> Result<(), DatasetTableError> {
        let mut insert = self
            .client
            .inner()
            .insert::<RecordRow>(table)
            .await?
            .with_timeouts(Some(self.insert.send), Some(self.insert.end));
        insert.write(row).await?;
        insert.end().await?;

        Ok(())
    }

    /// SAFETY: `order` is an expression, not a value, so it is written into
    /// the statement rather than bound. A caller only names a declared field;
    /// the expression itself is built by
    /// [`crate::domain::kinds::dataset::read`], whose `literal` escapes every
    /// declared path segment into a ClickHouse string literal — including the
    /// `?` that would otherwise shift this statement's bindings.
    pub(crate) async fn page(
        &self,
        table: &str,
        page: Page<'_>,
    ) -> Result<Vec<Record>, DatasetTableError> {
        let statement = READ_PAGE.replace("{order}", page.order);
        let rows = self
            .client
            .query(&statement)
            .bind(Identifier(table))
            .bind(page.limit)
            .bind(page.offset)
            .fetch_all::<PreviewRow>();
        let found = tokio::time::timeout(self.read_timeout, rows)
            .await
            .map_err(|_| DatasetTableError::Timeout)??;

        Ok(found.into_iter().map(PreviewRow::into_record).collect())
    }

    pub(crate) async fn count(&self, table: &str) -> Result<u64, DatasetTableError> {
        let rows = self
            .client
            .query(COUNT_ROWS)
            .bind(Identifier(table))
            .fetch_all::<CountRow>();
        let found = tokio::time::timeout(self.read_timeout, rows)
            .await
            .map_err(|_| DatasetTableError::Timeout)??;

        Ok(found.first().map_or(0, |row| row.total))
    }

    pub(crate) async fn shape_of(&self, table: &str) -> Result<Shape, DatasetTableError> {
        let shapes = self
            .client
            .inner()
            .query(READ_SHAPE)
            .bind(table)
            .fetch_all::<ShapeRow>();
        let found = tokio::time::timeout(self.read_timeout, shapes)
            .await
            .map_err(|_| DatasetTableError::Timeout)??;

        let Some(row) = found.first() else {
            return Ok(Shape::Absent);
        };

        if row.sorting_key == INGEST_SORTING_KEY {
            Ok(Shape::Ingest)
        } else {
            Ok(Shape::Foreign)
        }
    }

    pub(crate) async fn provision(&self, table: &str) -> Result<(), DatasetTableError> {
        let database = &self.client.config().database;
        tokio::time::timeout(
            self.read_timeout,
            self.client.assert_database_topology(database),
        )
        .await
        .map_err(|_| DatasetTableError::Timeout)??;

        match self.shape_of(table).await? {
            Shape::Foreign => return Err(DatasetTableError::NotOurs(table.to_owned())),
            Shape::Absent | Shape::Ingest => {}
        }

        let create = self.ingest_spec(table);
        tokio::time::timeout(self.ddl_timeout, self.client.execute_ddl(&create))
            .await
            .map_err(|_| DatasetTableError::Timeout)??;

        Ok(())
    }

    fn ingest_spec(&self, table: &str) -> Ddl {
        Ddl::create_table(
            TableSpec::merge_tree(&self.client.config().database, table, &COLUMNS, &SORT_KEY)
                .keep_existing(),
        )
    }

    pub(crate) async fn drop_table(&self, table: &str) -> Result<(), DatasetTableError> {
        let drop = Ddl::drop_table(&self.client.config().database, table);
        tokio::time::timeout(self.ddl_timeout, self.client.execute_ddl(&drop))
            .await
            .map_err(|_| DatasetTableError::Timeout)??;

        Ok(())
    }
}

impl fmt::Debug for DatasetTables {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DatasetTables")
            .field("ddl_timeout", &self.ddl_timeout)
            .field("read_timeout", &self.read_timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error)]
pub(crate) enum DatasetTableError {
    #[error("`{0}` is held by a table this service does not own")]
    NotOurs(String),
    #[error("the table a record was to land in is gone")]
    Vanished,
    #[error("the datasets database did not answer in time")]
    Timeout,
    #[error("the datasets database did not answer")]
    ClickHouse(clickhouse::error::Error),
    #[error("the dataset's table was aimed at a database this service does not own")]
    NotOurDatabase,
    #[error("the datasets database is not ready: {0}")]
    DatabaseTopology(String),
}

#[derive(Debug, Deserialize, Serialize, clickhouse::Row)]
struct ShapeRow {
    sorting_key: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Record {
    pub(crate) id: Uuid,
    pub(crate) received_at: DateTime<Utc>,
    pub(crate) raw_data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, clickhouse::Row)]
struct RecordRow {
    #[serde(with = "clickhouse::serde::uuid")]
    id: Uuid,
    table_name: String,
    raw_data: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    received_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, clickhouse::Row)]
struct CountRow {
    total: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Page<'a> {
    pub(crate) limit: u64,
    pub(crate) offset: u64,
    pub(crate) order: &'a str,
}

#[derive(Debug, Serialize, Deserialize, clickhouse::Row)]
struct PreviewRow {
    #[serde(with = "clickhouse::serde::uuid")]
    id: Uuid,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    received_at: DateTime<Utc>,
    raw_data: String,
}

impl PreviewRow {
    fn into_record(self) -> Record {
        let raw_data = serde_json::from_str(&self.raw_data)
            .unwrap_or_else(|_| serde_json::Value::String(self.raw_data.clone()));

        Record {
            id: self.id,
            received_at: self.received_at,
            raw_data,
        }
    }
}

impl From<clickhouse::error::Error> for DatasetTableError {
    fn from(error: clickhouse::error::Error) -> Self {
        match error {
            clickhouse::error::Error::TimedOut => Self::Timeout,
            ref reported if Unfinished::from_error(reported).is_some() => Self::Timeout,
            ref reported if MissingRelation::from_error(reported).is_some() => Self::Vanished,
            error => Self::ClickHouse(error),
        }
    }
}

impl From<insight_clickhouse::Error> for DatasetTableError {
    fn from(error: insight_clickhouse::Error) -> Self {
        match error {
            insight_clickhouse::Error::Clickhouse(reported) => Self::from(reported),
            insight_clickhouse::Error::Timeout => Self::Timeout,
            insight_clickhouse::Error::InvalidQuery(_) => Self::NotOurDatabase,
            insight_clickhouse::Error::Topology(remedy) => Self::DatabaseTopology(remedy),
        }
    }
}

#[cfg(test)]
mod tests;
