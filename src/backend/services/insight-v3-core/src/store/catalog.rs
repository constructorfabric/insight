//! The warehouse catalogue: every table the connected user can see, with its
//! columns, its engine and the layer its database says it belongs to.
//!
//! The datasets database is left out. A dataset is read through its
//! declaration, never as the table underneath it, so that table is not
//! offered as something a metric would name.

use std::fmt;
use std::time::{Duration, Instant};

use serde::Deserialize;
use thiserror::Error;
use tokio::sync::RwLock;

#[cfg(test)]
mod tests;

/// How long one listing serves before the warehouse is asked again. A table
/// made in between is still readable; it is only not listed yet.
const CACHE_TTL_SECS: u64 = 5 * 60;
const READ_TIMEOUT_SECS: u64 = 10;

const BRONZE_PREFIX: &str = "bronze_";
const SILVER_DATABASE: &str = "silver";
const IDENTITY_DATABASE: &str = "identity";

/// Every column of every table, in table order, with the engine that holds
/// the table. The engine's own databases are left out, and so is the one
/// datasets keep their records in, and a materialised view's own storage.
///
/// INVARIANT: the listing is derived from the columns, so a relation with no
/// column of its own is not in it. `ClickHouse` has none, and reading the
/// columns and the tables apart would cost a round trip to say the same.
const LIST_COLUMNS: &str = "SELECT c.database AS database, c.table AS table, c.name AS name, c.type AS type, t.engine AS engine
FROM system.columns AS c
INNER JOIN system.tables AS t ON t.database = c.database AND t.name = c.table
WHERE c.database NOT IN ('system', 'information_schema', 'INFORMATION_SCHEMA')
  AND c.database != ?
  AND NOT startsWith(c.table, '.inner')
ORDER BY c.database, c.table, c.position";

/// Which part of the warehouse a table belongs to, read off its database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Layer {
    Bronze,
    Silver,
    Gold,
    Identity,
    Other,
}

impl Layer {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Bronze => "bronze",
            Self::Silver => "silver",
            Self::Gold => "gold",
            Self::Identity => "identity",
            Self::Other => "other",
        }
    }
}

/// One table as a listing names it, without the columns a listing never
/// shows: reading those out of the cache would copy every string in it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TableEntry {
    pub(crate) database: String,
    pub(crate) table: String,
    pub(crate) layer: Layer,
}

/// One column, as `system.columns` names and types it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Column {
    pub(crate) name: String,
    pub(crate) kind: String,
}

/// The tables a description asked for, and how many there were to describe.
#[derive(Debug)]
pub(crate) struct Described {
    pub(crate) tables: Vec<TableSchema>,
    pub(crate) total: usize,
}

/// One table the warehouse holds, with everything a metric author needs to
/// name it and its columns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TableSchema {
    pub(crate) database: String,
    pub(crate) table: String,
    pub(crate) layer: Layer,
    /// The engine as `system.tables` spells it, `ReplacingMergeTree` and the
    /// like, so a reader can tell a table that is read through `FINAL`.
    pub(crate) engine: String,
    /// In the table's own column order.
    pub(crate) columns: Vec<Column>,
}

impl TableSchema {
    /// Whether `database.table`, or a bare table name in any database, means
    /// this table.
    pub(crate) fn is_named(&self, name: &str) -> bool {
        match name.split_once('.') {
            Some((database, table)) => self.database == database && self.table == table,
            None => self.table == name,
        }
    }

    #[cfg(test)]
    pub(crate) fn qualified(&self) -> String {
        format!("{}.{}", self.database, self.table)
    }
}

#[derive(Debug, clickhouse::Row, Deserialize)]
struct ColumnRow {
    database: String,
    table: String,
    name: String,
    r#type: String,
    engine: String,
}

pub(crate) struct Catalog {
    client: insight_clickhouse::Client,
    gold_database: String,
    datasets_database: String,
    read_timeout: Duration,
    cached: RwLock<Option<Cached>>,
}

#[derive(Debug)]
struct Cached {
    at: Instant,
    tables: Vec<TableSchema>,
}

impl Catalog {
    pub(crate) fn new(
        client: insight_clickhouse::Client,
        gold_database: &str,
        datasets_database: &str,
    ) -> Self {
        Self {
            client,
            gold_database: gold_database.to_owned(),
            datasets_database: datasets_database.to_owned(),
            read_timeout: Duration::from_secs(READ_TIMEOUT_SECS),
            cached: RwLock::new(None),
        }
    }

    /// A catalogue that already holds these tables and never asks the
    /// warehouse.
    #[cfg(test)]
    pub(crate) fn fixed(tables: Vec<TableSchema>) -> Self {
        let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://catalogue.invalid",
            "insight",
        ));
        Self {
            client,
            gold_database: "insight".to_owned(),
            datasets_database: "insight_datasets".to_owned(),
            read_timeout: Duration::from_secs(READ_TIMEOUT_SECS),
            cached: RwLock::new(Some(Cached {
                at: Instant::now(),
                tables,
            })),
        }
    }

    /// Every table the connected user can see, in database and table order,
    /// named but not spelled out.
    pub(crate) async fn tables(&self) -> Result<Vec<TableEntry>, CatalogError> {
        self.read(|tables| tables.iter().map(entry_of).collect())
            .await
    }

    /// The tables these names mean: `database.table`, or a bare table name,
    /// which means it in every database that has one.
    ///
    /// At most `most` of them: a bare name can mean a table in every database
    /// the warehouse holds, and each carries every column it has, so the cap
    /// belongs here rather than over what has already been copied out.
    pub(crate) async fn describe(
        &self,
        names: &[String],
        most: usize,
    ) -> Result<Described, CatalogError> {
        self.read(|tables| {
            let matching = tables
                .iter()
                .filter(|schema| names.iter().any(|name| schema.is_named(name)));
            let total = matching.clone().count();

            Described {
                tables: matching.take(most).cloned().collect(),
                total,
            }
        })
        .await
    }

    /// One table, if the warehouse has it.
    pub(crate) async fn find(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Option<TableSchema>, CatalogError> {
        self.read(|tables| {
            tables
                .iter()
                .find(|schema| schema.database == database && schema.table == table)
                .cloned()
        })
        .await
    }

    /// Answers `pick` over the cached listing, reloading it once it has aged
    /// out. Only what `pick` keeps is copied.
    ///
    /// INVARIANT: the reload happens under the write lock and re-checks the
    /// age, so a cold start or an expiry asks the warehouse once however many
    /// callers arrive together. The listing is the heaviest read this service
    /// makes; letting them all issue it is how a restart takes the warehouse
    /// down with it.
    async fn read<T>(&self, pick: impl Fn(&[TableSchema]) -> T) -> Result<T, CatalogError> {
        if let Some(picked) = self.fresh(&pick).await {
            return Ok(picked);
        }

        let mut cached = self.cached.write().await;
        if let Some(fresh) = cached.as_ref().filter(|held| Self::young(held)) {
            return Ok(pick(&fresh.tables));
        }

        let listed = self.load().await;
        let tables = match (listed, cached.take()) {
            (Ok(tables), _) => tables,
            // SAFETY: a warehouse that did not answer must not send every
            // caller behind this lock to wait on it in turn. What was last
            // listed serves another term instead - a schema catalogue a few
            // minutes stale is worth more than a queue of ten-second waits.
            (Err(error), Some(stale)) => {
                tracing::warn!(%error, "the warehouse catalogue could not be reloaded");
                stale.tables
            }
            (Err(error), None) => return Err(error),
        };
        let picked = pick(&tables);
        *cached = Some(Cached {
            at: Instant::now(),
            tables,
        });

        Ok(picked)
    }

    async fn fresh<T>(&self, pick: &impl Fn(&[TableSchema]) -> T) -> Option<T> {
        let cached = self.cached.read().await;
        let held = cached.as_ref().filter(|held| Self::young(held))?;

        Some(pick(&held.tables))
    }

    fn young(held: &Cached) -> bool {
        held.at.elapsed() < Duration::from_secs(CACHE_TTL_SECS)
    }

    async fn load(&self) -> Result<Vec<TableSchema>, CatalogError> {
        // SAFETY: through the wrapper, not `inner()`: it attaches this
        // installation's execution-time, thread and memory ceilings, and this
        // is the widest statement the service issues.
        let rows = self
            .client
            .query(LIST_COLUMNS)
            .bind(self.datasets_database.as_str())
            .fetch_all::<ColumnRow>();
        let rows = tokio::time::timeout(self.read_timeout, rows)
            .await
            .map_err(|_| CatalogError::Timeout)??;

        Ok(rows
            .chunk_by(|left, right| left.database == right.database && left.table == right.table)
            .filter_map(|one_table| schema_of(one_table, &self.gold_database))
            .collect())
    }
}

impl fmt::Debug for Catalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Catalog")
            .field("gold_database", &self.gold_database)
            .field("datasets_database", &self.datasets_database)
            .finish_non_exhaustive()
    }
}

fn entry_of(schema: &TableSchema) -> TableEntry {
    TableEntry {
        database: schema.database.clone(),
        table: schema.table.clone(),
        layer: schema.layer,
    }
}

fn schema_of(columns_of_one_table: &[ColumnRow], gold_database: &str) -> Option<TableSchema> {
    let [first, ..] = columns_of_one_table else {
        return None;
    };

    let columns = columns_of_one_table
        .iter()
        .map(|row| Column {
            name: row.name.clone(),
            kind: row.r#type.clone(),
        })
        .collect();

    Some(TableSchema {
        database: first.database.clone(),
        table: first.table.clone(),
        layer: classify(&first.database, gold_database),
        engine: first.engine.clone(),
        columns,
    })
}

fn classify(database: &str, gold_database: &str) -> Layer {
    if database.starts_with(BRONZE_PREFIX) {
        return Layer::Bronze;
    }
    match database {
        SILVER_DATABASE => Layer::Silver,
        IDENTITY_DATABASE => Layer::Identity,
        _ if database == gold_database => Layer::Gold,
        _ => Layer::Other,
    }
}

#[derive(Debug, Error)]
pub(crate) enum CatalogError {
    #[error("the warehouse catalogue could not be read")]
    ClickHouse(#[from] clickhouse::error::Error),
    #[error("the warehouse catalogue did not answer in time")]
    Timeout,
}
