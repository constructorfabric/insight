//! What the assistant is told before it is asked anything.

use std::fmt::Write as _;

use super::definition::{DefinitionKind, Definitions};
use crate::chat::{Catalogue, KnownTable, Schemas};
use crate::store::catalog::{Catalog, Layer, TableSchema};
use crate::store::tables::{TableName, TableStore};

/// Everything the model is given about this stand, gathered once per ask.
#[derive(Debug, Default)]
pub(crate) struct Briefing {
    /// The tables data has been ingested into, with the fields they hold.
    pub(crate) tables: Vec<KnownTable>,
    /// What is already built, so the model can name and replace it.
    pub(crate) catalogue: Catalogue,
    /// Every table on the stand by layer, names only.
    pub(crate) map: String,
    /// Every table a query may name, qualified as it must be named.
    pub(crate) allowed: Vec<String>,
}

/// Reads the stand for what the assistant needs to know about it.
#[derive(Debug)]
pub(crate) struct Assistant<'a> {
    definitions: &'a dyn Definitions,
    catalog: &'a Catalog,
    tables: &'a TableStore,
}

impl<'a> Assistant<'a> {
    pub(crate) fn new(
        definitions: &'a dyn Definitions,
        catalog: &'a Catalog,
        tables: &'a TableStore,
    ) -> Self {
        Self {
            definitions,
            catalog,
            tables,
        }
    }

    /// Everything the model is told, read fresh: a stand gains tables and
    /// definitions between one question and the next.
    pub(crate) async fn briefing(&self) -> Briefing {
        let tables = self.known_tables().await;
        let allowed = self.queryable_tables(&tables).await;

        Briefing {
            catalogue: self.catalogue().await,
            map: self.layer_map().await,
            allowed,
            tables,
        }
    }

    /// Every table a query may name, as it must name it: `database.table` for a
    /// table in a layer, and the bare name for one ingested here, which a stored
    /// metric has always addressed without a database.
    ///
    /// This is what stops the model querying a table it invented - it reached for
    /// `information_schema` when it had nothing else - while letting it reach
    /// every real table on the stand.
    async fn queryable_tables(&self, ingested: &[KnownTable]) -> Vec<String> {
        let mut allowed: Vec<String> = ingested.iter().map(|table| table.name.clone()).collect();

        match self.catalog.tables().await {
            Ok(tables) => allowed.extend(
                tables
                    .iter()
                    .map(|table| format!("{}.{}", table.database, table.table)),
            ),
            Err(error) => {
                tracing::warn!(error = ?error, "could not list the stand's tables for the chat");
            }
        }

        allowed
    }

    /// Every table on the stand, grouped by layer, names only.
    ///
    /// The map is what lets the model reach bronze, silver, gold and identity
    /// without a hardcoded list: it is read from the stand each time, so a
    /// database added on another stand appears with no code change. Columns are
    /// left out on purpose - they run to tens of thousands of tokens - and the
    /// model asks for the ones it needs through `look_up`.
    async fn layer_map(&self) -> String {
        let tables = match self.catalog.tables().await {
            Ok(tables) => tables,
            Err(error) => {
                tracing::warn!(error = ?error, "could not map the stand for the chat");
                return String::new();
            }
        };

        let mut rendered = String::new();
        for (layer, label) in [
            (Layer::Gold, "Gold (published metrics)"),
            (Layer::Silver, "Silver (cleaned per-source models)"),
            (Layer::Identity, "Identity (who people are)"),
            (Layer::Bronze, "Bronze (raw provider payloads)"),
            (Layer::Ingest, "Ingested here (one JSON payload column)"),
        ] {
            let of_layer: Vec<&TableSchema> =
                tables.iter().filter(|table| table.layer == layer).collect();
            if of_layer.is_empty() {
                continue;
            }

            rendered.push_str(label);
            rendered.push('\n');
            // Grouped by database, because that is what a query has to name.
            let mut database = "";
            for table in of_layer {
                if table.database != database {
                    database = &table.database;
                    let _ = writeln!(rendered, "  {database}:");
                }
                let _ = writeln!(rendered, "    {}", table.table);
            }
            rendered.push('\n');
        }

        rendered
    }

    /// What is already stored, so the model can name it, reuse it, and replace it
    /// when the reader asks for a change. A listing failure degrades the hint; it
    /// does not fail the chat.
    async fn catalogue(&self) -> Catalogue {
        let mut built = Vec::with_capacity(DefinitionKind::ALL.len());

        for kind in DefinitionKind::ALL {
            built.push((kind, self.names(kind).await));
        }

        Catalogue::new(built)
    }

    async fn names(&self, kind: DefinitionKind) -> Vec<String> {
        match self.definitions.list(kind).await {
            Ok(names) => names,
            Err(error) => {
                tracing::warn!(error = ?error, ?kind, "could not list definitions for the chat");
                Vec::new()
            }
        }
    }

    /// The tables the reader has data in, each with the field names and types
    /// `TableStore::sample_fields` found in its most recent rows. A table with
    /// nothing in it is left out.
    ///
    /// Read from the ingested tables themselves, so a stand with no metrics yet
    /// still tells the model what data exists. A listing failure degrades the
    /// hint; it does not fail the chat.
    async fn known_tables(&self) -> Vec<KnownTable> {
        let names = match self.tables.list().await {
            Ok(names) => names,
            Err(error) => {
                tracing::warn!(error = ?error, "could not list tables to seed chat table hints");
                return Vec::new();
            }
        };

        let mut described = Vec::with_capacity(names.len());
        for name in names {
            let Ok(table_name) = TableName::parse(&name) else {
                continue;
            };
            let fields = self
                .tables
                .sample_fields(&table_name)
                .await
                .unwrap_or_default();

            // Nothing has landed here, so there are no fields to query and
            // naming it only crowds the list the reader is shown.
            if fields.is_empty() {
                continue;
            }

            described.push(KnownTable {
                fields: fields
                    .iter()
                    .map(|(field, kind)| format!("{field} ({kind})"))
                    .collect::<Vec<_>>()
                    .join(", "),
                name,
            });
        }

        described
    }
}

/// The columns of the tables the model asked about, read from the same
/// listing the map came from.
#[derive(Debug)]
pub(crate) struct CatalogSchemas<'a> {
    catalog: &'a Catalog,
}

impl<'a> CatalogSchemas<'a> {
    pub(crate) fn new(catalog: &'a Catalog) -> Self {
        Self { catalog }
    }
}

#[async_trait::async_trait]
impl Schemas for CatalogSchemas<'_> {
    async fn describe(&self, tables: &[String]) -> String {
        let found = match self.catalog.describe(tables).await {
            Ok(found) => found,
            Err(error) => {
                tracing::warn!(error = ?error, "a schema lookup failed");
                return "The schema could not be read. Answer from the map alone.".to_owned();
            }
        };

        let mut rendered = String::new();
        for table in &found {
            let _ = writeln!(rendered, "{}.{}", table.database, table.table);
            for (column, kind) in &table.columns {
                let _ = writeln!(rendered, "  {column} {kind}");
            }
        }

        // A name that resolved to nothing is said so rather than left out:
        // silence reads as "no columns" and the model invents them.
        for asked in tables {
            let matched = found.iter().any(|table| {
                asked == &format!("{}.{}", table.database, table.table) || asked == &table.table
            });
            if !matched {
                let _ = writeln!(rendered, "{asked}: no such table on this stand");
            }
        }

        rendered
    }
}
