//! The tables a dataset's records live in.
//!
//! Every one of them is in the datasets database and nowhere else, so nothing
//! this service creates or drops can reach a table the warehouse owns.

use std::fmt;
use std::time::Duration;

use clickhouse::sql::Identifier;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
const READ_SHAPE: &str = "SELECT sorting_key FROM system.tables
WHERE database = currentDatabase() AND name = ?";
/// Creating or dropping a table waits on a cluster-wide lock, so it gets a
/// wider bound than a read.
const DDL_TIMEOUT_SECS: u64 = 35;
const READ_TIMEOUT_SECS: u64 = 10;

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
}

impl DatasetTables {
    pub(crate) fn new(client: insight_clickhouse::Client) -> Self {
        Self {
            client,
            ddl_timeout: Duration::from_secs(DDL_TIMEOUT_SECS),
            read_timeout: Duration::from_secs(READ_TIMEOUT_SECS),
        }
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
    #[error("the datasets database did not answer in time")]
    Timeout,
    #[error("the datasets database did not answer")]
    ClickHouse(#[from] clickhouse::error::Error),
}

#[derive(Debug, Deserialize, Serialize, clickhouse::Row)]
struct ShapeRow {
    sorting_key: String,
}

#[cfg(test)]
mod tests;
