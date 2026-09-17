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
/// The preview: the latest records, newest first, with the id breaking an
/// equal receipt instant so that repeated reads agree on the order.
const READ_LATEST: &str = "SELECT id, received_at, raw_data FROM ?
ORDER BY received_at DESC, id DESC
LIMIT ?";
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

    /// The latest records a dataset holds, newest first.
    ///
    /// Each record is given back as it was sent: this is a reader looking at
    /// what arrived, so a payload is never reshaped on the way out.
    pub(crate) async fn latest(
        &self,
        table: &str,
        limit: u64,
    ) -> Result<Vec<Record>, DatasetTableError> {
        let rows = self
            .client
            .inner()
            .query(READ_LATEST)
            .bind(Identifier(table))
            .bind(limit)
            .fetch_all::<PreviewRow>();
        let found = tokio::time::timeout(self.read_timeout, rows)
            .await
            .map_err(|_| DatasetTableError::Timeout)??;

        Ok(found.into_iter().map(PreviewRow::into_record).collect())
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

/// One record as a dataset's table holds it: whole, and stamped with when it
/// arrived.
/// One stored record as a reader is shown it.
#[derive(Debug, Serialize)]
pub(crate) struct Record {
    pub(crate) id: Uuid,
    pub(crate) received_at: DateTime<Utc>,
    /// The payload as it arrived. A record this service could not read back
    /// as JSON is given as the text it holds, rather than left out.
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
