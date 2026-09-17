//! The stand's table catalogue: every database the connected user can see,
//! with each table's columns and the layer it belongs to.

use std::fmt;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::RwLock;

const CACHE_TTL: Duration = Duration::from_mins(5);
const BRONZE_PREFIX: &str = "bronze_";
const SILVER_DATABASE: &str = "silver";
const IDENTITY_DATABASE: &str = "identity";
// INVARIANT: mirrors the ingest schema in `tables::CREATE_TABLE`.
const INGEST_COLUMNS: [&str; 4] = ["id", "table_name", "raw_data", "received_at"];
const LIST_COLUMNS: &str = "SELECT c.database AS database, c.table AS table, c.name AS name, c.type AS type, t.engine AS engine
FROM system.columns AS c
INNER JOIN system.tables AS t ON t.database = c.database AND t.name = c.table
WHERE c.database NOT IN ('system', 'information_schema', 'INFORMATION_SCHEMA', 'default')
ORDER BY c.database, c.table, c.position";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Layer {
    Bronze,
    Silver,
    Gold,
    Identity,
    Ingest,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableEngine {
    MergeTree,
    ReplacingMergeTree,
    Other,
}

impl TableEngine {
    fn parse(value: &str) -> Self {
        if value.ends_with("ReplacingMergeTree") {
            return Self::ReplacingMergeTree;
        }
        if value.ends_with("MergeTree") {
            return Self::MergeTree;
        }

        Self::Other
    }

    pub(crate) fn requires_final(self) -> bool {
        self == Self::ReplacingMergeTree
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TableSchema {
    pub(crate) database: String,
    pub(crate) table: String,
    pub(crate) layer: Layer,
    pub(crate) engine: TableEngine,
    /// (column, `ClickHouse` type), in the table's own order.
    pub(crate) columns: Vec<(String, String)>,
}

impl TableSchema {
    fn is_named(&self, name: &str) -> bool {
        match name.split_once('.') {
            Some((database, table)) => self.database == database && self.table == table,
            None => self.table == name,
        }
    }
}

#[derive(Debug, clickhouse::Row, serde::Deserialize)]
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
    cached: RwLock<Option<Cached>>,
}

#[cfg(test)]
impl Catalog {
    /// A catalogue that already holds these tables and never reloads.
    pub(crate) fn fixed(tables: Vec<TableSchema>) -> Self {
        Self {
            client: insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                "http://catalogue.invalid",
                "insight",
            )),
            gold_database: "insight".to_owned(),
            cached: RwLock::new(Some(Cached {
                at: Instant::now(),
                tables,
            })),
        }
    }
}

#[derive(Debug)]
struct Cached {
    at: Instant,
    tables: Vec<TableSchema>,
}

impl Catalog {
    pub(crate) fn new(client: insight_clickhouse::Client, gold_database: String) -> Self {
        Self {
            client,
            gold_database,
            cached: RwLock::new(None),
        }
    }

    /// Every table the connected user can see, cached for `CACHE_TTL`.
    pub(crate) async fn tables(&self) -> Result<Vec<TableSchema>, CatalogError> {
        self.read(<[TableSchema]>::to_vec).await
    }

    /// Which engine holds one table, and `Other` for a table this catalogue
    /// has never heard of.
    pub(crate) async fn engine_of(&self, name: &str) -> Result<TableEngine, CatalogError> {
        self.read(|tables| {
            tables
                .iter()
                .find(|schema| schema.is_named(name))
                .map_or(TableEngine::Other, |schema| schema.engine)
        })
        .await
    }

    /// Answers `pick` over the cached catalogue, reloading it when the cache
    /// has aged out. Only what `pick` keeps is copied.
    async fn read<T>(&self, pick: impl Fn(&[TableSchema]) -> T) -> Result<T, CatalogError> {
        {
            let cached = self.cached.read().await;
            if let Some(cached) = cached
                .as_ref()
                .filter(|cached| cached.at.elapsed() < CACHE_TTL)
            {
                return Ok(pick(&cached.tables));
            }
        }

        let tables = self.load().await?;
        let picked = pick(&tables);
        *self.cached.write().await = Some(Cached {
            at: Instant::now(),
            tables,
        });

        Ok(picked)
    }

    async fn load(&self) -> Result<Vec<TableSchema>, CatalogError> {
        let rows = self
            .client
            .inner()
            .query(LIST_COLUMNS)
            .fetch_all::<ColumnRow>()
            .await?;

        Ok(rows
            .chunk_by(|left, right| left.database == right.database && left.table == right.table)
            .filter_map(|chunk| schema_of(chunk, &self.gold_database))
            .collect())
    }
}

impl fmt::Debug for Catalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Catalog")
            .field("gold_database", &self.gold_database)
            .finish_non_exhaustive()
    }
}

fn schema_of(columns_of_one_table: &[ColumnRow], gold_database: &str) -> Option<TableSchema> {
    let [first, ..] = columns_of_one_table else {
        return None;
    };
    let columns: Vec<(String, String)> = columns_of_one_table
        .iter()
        .map(|row| (row.name.clone(), row.r#type.clone()))
        .collect();

    Some(TableSchema {
        database: first.database.clone(),
        table: first.table.clone(),
        layer: classify(&first.database, gold_database, &columns),
        engine: TableEngine::parse(&first.engine),
        columns,
    })
}

fn classify(database: &str, gold_database: &str, columns: &[(String, String)]) -> Layer {
    if is_ingest_schema(columns) {
        return Layer::Ingest;
    }
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

fn is_ingest_schema(columns: &[(String, String)]) -> bool {
    columns.len() == INGEST_COLUMNS.len()
        && INGEST_COLUMNS
            .iter()
            .all(|wanted| columns.iter().any(|(name, _)| name == wanted))
}

#[derive(Debug, Error)]
pub(crate) enum CatalogError {
    #[error("table catalogue lookup failed")]
    ClickHouse(#[from] clickhouse::error::Error),
}

#[cfg(test)]
mod tests;
