//! What a relation the warehouse builds looks like, as a dataset over one has
//! to know.
//!
//! This reads through the query connection, which is read-only and reaches
//! every database. Nothing here writes, and nothing here can: the connection
//! that may create or drop a table is bound to the datasets database and is
//! not this one.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

const READ_COLUMNS: &str = "SELECT name, type FROM system.columns
WHERE database = ? AND table = ?
ORDER BY position";
const READ_ENGINE: &str = "SELECT engine FROM system.tables
WHERE database = ? AND name = ?";
const READ_TIMEOUT_SECS: u64 = 10;

/// The engine families that keep superseded rows until a merge takes them
/// away, so that a plain read counts one row more than once.
///
/// INVARIANT: an engine that does not collapse refuses `FINAL` outright
/// (`ILLEGAL_FINAL`), so this cannot be widened into "ask for it everywhere
/// and let the rest ignore it".
const COLLAPSING: [&str; 5] = [
    "ReplacingMergeTree",
    "CollapsingMergeTree",
    "VersionedCollapsingMergeTree",
    "SummingMergeTree",
    "AggregatingMergeTree",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Column {
    pub(crate) name: String,
    /// The warehouse's own type, as it writes it.
    pub(crate) held: String,
}

pub(crate) struct Relations {
    client: insight_clickhouse::Client,
    read_timeout: Duration,
}

impl Relations {
    pub(crate) fn new(client: insight_clickhouse::Client) -> Self {
        Self {
            client,
            read_timeout: Duration::from_secs(READ_TIMEOUT_SECS),
        }
    }

    /// Whether a read of this relation would count a superseded row again.
    pub(crate) async fn collapses(
        &self,
        database: &str,
        table: &str,
    ) -> Result<bool, RelationError> {
        let reading = self
            .client
            .inner()
            .query(READ_ENGINE)
            .bind(database)
            .bind(table)
            .fetch_all::<EngineRow>();
        let rows = tokio::time::timeout(self.read_timeout, reading)
            .await
            .map_err(|_| RelationError::Timeout)??;

        Ok(rows.first().is_some_and(|row| collapses(&row.engine)))
    }

    /// The columns this relation holds, in the order it holds them.
    ///
    /// An empty answer is a relation the warehouse does not have: nothing
    /// distinguishes a relation with no columns from one that is not there,
    /// and neither can be read.
    pub(crate) async fn columns(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<Column>, RelationError> {
        let reading = self
            .client
            .inner()
            .query(READ_COLUMNS)
            .bind(database)
            .bind(table)
            .fetch_all::<ColumnRow>();
        let rows = tokio::time::timeout(self.read_timeout, reading)
            .await
            .map_err(|_| RelationError::Timeout)??;

        Ok(rows
            .into_iter()
            .map(|row| Column {
                name: row.name,
                held: row.r#type,
            })
            .collect())
    }
}

/// Whether a relation on this engine keeps rows a later merge will remove.
///
/// A view is left alone: whether its rows need collapsing is decided by the
/// select it is made of, which is where that belongs.
fn collapses(engine: &str) -> bool {
    let family = engine
        .trim_start_matches("Shared")
        .trim_start_matches("Replicated");

    COLLAPSING.contains(&family)
}

impl fmt::Debug for Relations {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Relations")
            .field("read_timeout", &self.read_timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Deserialize, Serialize, clickhouse::Row)]
struct EngineRow {
    engine: String,
}

#[derive(Debug, Deserialize, Serialize, clickhouse::Row)]
struct ColumnRow {
    name: String,
    r#type: String,
}

#[derive(Debug, Error)]
pub(crate) enum RelationError {
    #[error("the warehouse did not answer in time")]
    Timeout,
    #[error(transparent)]
    Warehouse(#[from] clickhouse::error::Error),
}

#[cfg(test)]
mod tests;
