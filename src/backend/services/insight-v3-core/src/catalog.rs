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

    /// The named tables only, for a schema lookup. `database.table` or a bare
    /// table, which matches in any database.
    pub(crate) async fn describe(
        &self,
        names: &[String],
    ) -> Result<Vec<TableSchema>, CatalogError> {
        self.read(|tables| {
            tables
                .iter()
                .filter(|schema| names.iter().any(|name| schema.is_named(name)))
                .cloned()
                .collect()
        })
        .await
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
mod tests {
    use clickhouse::test::{Mock, handlers};

    use super::*;

    #[derive(clickhouse::Row, serde::Serialize)]
    struct ColumnFixture {
        database: String,
        table: String,
        name: String,
        r#type: String,
        engine: String,
    }

    fn column(database: &str, table: &str, name: &str, kind: &str) -> ColumnFixture {
        ColumnFixture {
            database: database.to_owned(),
            table: table.to_owned(),
            name: name.to_owned(),
            r#type: kind.to_owned(),
            engine: "MergeTree".to_owned(),
        }
    }

    fn columns(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|name| ((*name).to_owned(), "String".to_owned()))
            .collect()
    }

    fn catalog_over(rows: Vec<ColumnFixture>) -> (Mock, Catalog) {
        let mock = Mock::new();
        mock.add(handlers::provide(rows));
        let client =
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));

        (mock, Catalog::new(client, "insight".to_owned()))
    }

    async fn listing(catalog: &Catalog) -> Vec<TableSchema> {
        catalog
            .tables()
            .await
            .unwrap_or_else(|error| panic!("the catalogue should list: {error}"))
    }

    #[test]
    fn the_listing_leaves_out_the_engines_own_databases() {
        assert!(LIST_COLUMNS.contains("system.columns"));

        for database in [
            "'system'",
            "'information_schema'",
            "'INFORMATION_SCHEMA'",
            "'default'",
        ] {
            assert!(
                LIST_COLUMNS.contains(database),
                "should exclude {database}: {LIST_COLUMNS}"
            );
        }

        assert!(LIST_COLUMNS.contains("ORDER BY c.database, c.table, c.position"));
    }

    #[test]
    fn a_layer_is_read_off_the_database_name() {
        let cases = [
            ("bronze_github", Layer::Bronze),
            ("bronze_whatever", Layer::Bronze),
            ("bronze", Layer::Other),
            ("silver", Layer::Silver),
            ("identity", Layer::Identity),
            ("insight", Layer::Gold),
            ("staging", Layer::Other),
        ];

        for (database, expected) in cases {
            assert_eq!(
                classify(database, "insight", &columns(&["day"])),
                expected,
                "should classify {database}"
            );
        }
    }

    #[test]
    fn the_gold_layer_follows_the_configured_database() {
        let day = columns(&["day"]);

        assert_eq!(classify("warehouse", "warehouse", &day), Layer::Gold);
        assert_eq!(classify("insight", "warehouse", &day), Layer::Other);
    }

    #[test]
    fn a_table_carrying_the_ingest_schema_is_not_its_databases_layer() {
        let ingest = columns(&["id", "table_name", "raw_data", "received_at"]);
        let partial = columns(&["id", "table_name", "raw_data"]);

        assert_eq!(classify("insight", "insight", &ingest), Layer::Ingest);
        assert_eq!(classify("insight", "insight", &partial), Layer::Gold);
    }

    #[tokio::test]
    async fn each_table_becomes_one_schema_carrying_its_columns_in_position_order() {
        let (_mock, catalog) = catalog_over(vec![
            column("silver", "git_commits", "sha", "String"),
            column("silver", "git_commits", "lines_changed", "UInt32"),
            column("silver", "git_reviews", "reviewer", "String"),
        ]);

        let tables = listing(&catalog).await;

        assert_eq!(
            tables,
            vec![
                TableSchema {
                    database: "silver".to_owned(),
                    table: "git_commits".to_owned(),
                    layer: Layer::Silver,
                    engine: TableEngine::MergeTree,
                    columns: vec![
                        ("sha".to_owned(), "String".to_owned()),
                        ("lines_changed".to_owned(), "UInt32".to_owned()),
                    ],
                },
                TableSchema {
                    database: "silver".to_owned(),
                    table: "git_reviews".to_owned(),
                    layer: Layer::Silver,
                    engine: TableEngine::MergeTree,
                    columns: vec![("reviewer".to_owned(), "String".to_owned())],
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_bare_table_is_described_in_every_database_that_has_one() {
        let (_mock, catalog) = catalog_over(vec![
            column("bronze_github", "issues", "number", "UInt64"),
            column("silver", "git_commits", "sha", "String"),
            column("silver", "issues", "key", "String"),
        ]);

        let named = catalog
            .describe(&[
                "silver.git_commits".to_owned(),
                "issues".to_owned(),
                "not_on_this_stand".to_owned(),
            ])
            .await
            .unwrap_or_else(|error| panic!("the catalogue should describe: {error}"));

        let found: Vec<(String, String)> = named
            .into_iter()
            .map(|schema| (schema.database, schema.table))
            .collect();
        assert_eq!(
            found,
            vec![
                ("bronze_github".to_owned(), "issues".to_owned()),
                ("silver".to_owned(), "git_commits".to_owned()),
                ("silver".to_owned(), "issues".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn a_second_listing_inside_the_ttl_asks_clickhouse_nothing() {
        // The mock answers one request, so a second query fails the call below.
        let (_mock, catalog) = catalog_over(vec![column("silver", "git_commits", "sha", "String")]);

        let first = listing(&catalog).await;
        let second = catalog
            .tables()
            .await
            .unwrap_or_else(|error| panic!("the cached listing should not query again: {error}"));

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn a_schema_carries_the_engine_loaded_with_its_columns() {
        let mut replacing = column("silver", "events", "id", "UInt64");
        replacing.engine = "ReplicatedReplacingMergeTree".to_owned();
        let (_mock, catalog) = catalog_over(vec![replacing]);

        let tables = listing(&catalog).await;

        assert_eq!(tables[0].engine, TableEngine::ReplacingMergeTree);
        assert!(tables[0].engine.requires_final());
        assert!(!TableEngine::MergeTree.requires_final());
    }
}
