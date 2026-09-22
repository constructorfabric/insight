//! The tables a dataset's records live in.
//!
//! Every one of them is in the datasets database and nowhere else, so nothing
//! this service creates or drops can reach a table the warehouse owns.

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use clickhouse::sql::Identifier;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// The shape every dataset's table has. A table of any other shape in the
/// datasets database belongs to something else and is never touched.
const INGEST_SORTING_KEY: &str = "table_name, received_at, id";
const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS ? (
    id UUID,
    table_name String,
    raw_data String,
    received_at DateTime64(3, 'UTC')
)
ENGINE = MergeTree
ORDER BY (table_name, received_at, id)";
const DROP_TABLE: &str = "DROP TABLE IF EXISTS ?";
/// One page of records. The id breaks an equal ordering value so that two
/// reads of the same page agree on what is in it.
const READ_PAGE: &str = "SELECT id, received_at, raw_data FROM ?
ORDER BY {order}, received_at DESC, id DESC
LIMIT ? OFFSET ?";
const COUNT_ROWS: &str = "SELECT count() AS total FROM ?";
const READ_SHAPE: &str = "SELECT sorting_key FROM system.tables
WHERE database = currentDatabase() AND name = ?";
/// Creating or dropping a table waits on a cluster-wide lock, so it gets a
/// wider bound than a read.
const DDL_TIMEOUT_SECS: u64 = 35;
const READ_TIMEOUT_SECS: u64 = 10;
const INSERT_SEND_TIMEOUT_SECS: u64 = 10;
const INSERT_END_TIMEOUT_SECS: u64 = 30;
const INSERT_TOTAL_TIMEOUT_SECS: u64 = 35;
/// `ClickHouse` reports a missing relation as error 60 inside a message rather
/// than as a variant, and only the code is stable across a real server's text
/// and a header-only reply.
const UNKNOWN_TABLE_CODE: &str = "Code: 60";

/// What a table of that name in the datasets database is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// Nothing holds the name.
    Absent,
    /// A table this service could have made: the ingest shape.
    Ingest,
    /// A table of some other shape. Never created over, never dropped.
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
            ddl_timeout: Duration::from_secs(DDL_TIMEOUT_SECS),
            read_timeout: Duration::from_secs(READ_TIMEOUT_SECS),
            insert: InsertTimeouts {
                send: Duration::from_secs(INSERT_SEND_TIMEOUT_SECS),
                end: Duration::from_secs(INSERT_END_TIMEOUT_SECS),
                total: Duration::from_secs(INSERT_TOTAL_TIMEOUT_SECS),
            },
        }
    }

    /// Puts one record into a dataset's table, whole and unread.
    ///
    /// The table is never created here: a record can only land in a dataset
    /// somebody declared, and a table that is gone means the dataset went with
    /// it while this record was in flight.
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

    /// One page of a dataset's records.
    ///
    /// Each record is given back as it was sent: this is a reader looking at
    /// what arrived, so a payload is never reshaped on the way out.
    ///
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
            .inner()
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

    /// How many records the table holds, as they arrived: a re-sent record
    /// counts again here, since this is the count of what arrived.
    pub(crate) async fn count(&self, table: &str) -> Result<u64, DatasetTableError> {
        let rows = self
            .client
            .inner()
            .query(COUNT_ROWS)
            .bind(Identifier(table))
            .fetch_all::<CountRow>();
        let found = tokio::time::timeout(self.read_timeout, rows)
            .await
            .map_err(|_| DatasetTableError::Timeout)??;

        Ok(found.first().map_or(0, |row| row.total))
    }

    /// What, if anything, holds this name in the datasets database.
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

    /// Makes the table this attempt's records go into.
    ///
    /// Ownership is decided by shape alone: the declaration that will name
    /// this table is written only once the create finishes, so there is
    /// nothing else to ask. A table of the ingest shape already there is this
    /// attempt repeating itself.
    pub(crate) async fn provision(&self, table: &str) -> Result<(), DatasetTableError> {
        match self.shape_of(table).await? {
            Shape::Foreign => return Err(DatasetTableError::NotOurs(table.to_owned())),
            Shape::Absent | Shape::Ingest => {}
        }

        let create = self
            .client
            .inner()
            .query(CREATE_TABLE)
            .bind(Identifier(table));
        tokio::time::timeout(self.ddl_timeout, create.execute())
            .await
            .map_err(|_| DatasetTableError::Timeout)??;

        Ok(())
    }

    /// Drops a table this service made.
    ///
    /// The caller has already decided the table is ours: either a stored
    /// declaration names it, or this attempt made it moments ago and lost the
    /// dataset before anything could point at it.
    pub(crate) async fn drop_table(&self, table: &str) -> Result<(), DatasetTableError> {
        let drop = self
            .client
            .inner()
            .query(DROP_TABLE)
            .bind(Identifier(table));
        tokio::time::timeout(self.ddl_timeout, drop.execute())
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
}

#[derive(Debug, Deserialize, Serialize, clickhouse::Row)]
struct ShapeRow {
    sorting_key: String,
}

/// One row of a dataset as a reader is shown it.
///
/// A record sent into a dataset carries its own identity and the instant it
/// arrived. A row of a relation the warehouse builds carries neither: it was
/// not sent, and nothing here stamped it. Both are shown through the fields
/// the dataset declares, which is what `raw_data` holds.
#[derive(Debug, Serialize)]
pub(crate) struct Record {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) received_at: Option<DateTime<Utc>>,
    /// The values behind the row. For a record sent in, the payload as it
    /// arrived; for a row of a relation, its declared columns.
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

/// What a reader asked for: how many records, from where, in what order.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Page<'a> {
    pub(crate) limit: u64,
    pub(crate) offset: u64,
    /// The ordering expression, with its direction.
    pub(crate) order: &'a str,
}

/// A record as the preview reads it back: the columns the preview selects,
/// in the order it selects them.
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
            id: Some(self.id),
            received_at: Some(self.received_at),
            raw_data,
        }
    }
}

impl From<clickhouse::error::Error> for DatasetTableError {
    fn from(error: clickhouse::error::Error) -> Self {
        match error {
            clickhouse::error::Error::TimedOut => Self::Timeout,
            clickhouse::error::Error::BadResponse(ref message)
                if message.contains(UNKNOWN_TABLE_CODE) =>
            {
                Self::Vanished
            }
            error => Self::ClickHouse(error),
        }
    }
}

#[cfg(test)]
mod tests;
