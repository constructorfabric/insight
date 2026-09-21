//! What a relation the warehouse builds looks like, as a dataset over one has
//! to know.
//!
//! This reads through the query connection, which is read-only and reaches
//! every database. Nothing here writes, and nothing here can: the connection
//! that may create or drop a table is bound to the datasets database and is
//! not this one.

use std::fmt;
use std::time::Duration;

use clickhouse::sql::Identifier;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const READ_COLUMNS: &str = "SELECT name, type FROM system.columns
WHERE database = ? AND table = ?
ORDER BY position";
const READ_ENGINE: &str = "SELECT engine FROM system.tables
WHERE database = ? AND name = ?";
const COUNT_ROWS: &str = "SELECT count() AS total FROM ?";
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

    /// A few rows of this relation, each as an object of the fields a
    /// dataset declares over it.
    ///
    /// Values come back as text. A reader looking at rows is checking that
    /// what the warehouse holds is what they meant, and one text form per
    /// value says that without a column type per field having to survive the
    /// round trip.
    pub(crate) async fn rows(
        &self,
        database: &str,
        table: &str,
        reads: &[(String, String)],
        limit: u64,
    ) -> Result<Vec<Value>, RelationError> {
        if reads.is_empty() {
            return Ok(Vec::new());
        }

        // SAFETY: every part of this is built from the declaration, never
        // from what a caller typed - the names through `DefinitionName`, the
        // expressions through the dataset's own reader, whose `literal`
        // escapes what a declared path or column holds.
        let pairs: Vec<String> = reads
            .iter()
            .map(|(named, expression)| format!("{}, toString({expression})", literal(named)))
            .collect();
        let statement = format!(
            "SELECT toJSONString(map({})) AS row FROM ?.? LIMIT ?",
            pairs.join(", ")
        );

        let reading = self
            .client
            .inner()
            .query(&statement)
            .bind(Identifier(database))
            .bind(Identifier(table))
            .bind(limit)
            .fetch_all::<RowText>();
        let rows = tokio::time::timeout(self.read_timeout, reading)
            .await
            .map_err(|_| RelationError::Timeout)??;

        Ok(rows
            .into_iter()
            .map(|row| {
                serde_json::from_str(&row.row).unwrap_or(Value::Object(serde_json::Map::new()))
            })
            .collect())
    }

    /// How many rows this relation holds.
    pub(crate) async fn count(&self, database: &str, table: &str) -> Result<u64, RelationError> {
        let counting = self
            .client
            .inner()
            .query(COUNT_ROWS)
            .bind(Identifier(&format!("{database}.{table}")))
            .fetch_all::<Counted>();
        let rows = tokio::time::timeout(self.read_timeout, counting)
            .await
            .map_err(|_| RelationError::Timeout)??;

        Ok(rows.first().map_or(0, |row| row.total))
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

/// A value as a string literal the warehouse reads back unchanged.
///
/// WORKAROUND: the ClickHouse client scans a statement for `?` to find its
/// bind sites, so a `?` inside a literal would swallow the next bound value.
/// `??` is its escape and emits a single `?`.
fn literal(value: &str) -> String {
    format!(
        "'{}'",
        value
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('?', "??")
    )
}

#[derive(Debug, Deserialize, Serialize, clickhouse::Row)]
struct RowText {
    row: String,
}

#[derive(Debug, Deserialize, Serialize, clickhouse::Row)]
struct Counted {
    total: u64,
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
