//! Every statement a service emits that changes what the data layer holds.
//!
//! Two facts of a `Replicated` database decide the shape rendered here. It
//! replays each DDL on every replica but does not replicate rows: a table
//! replicates only when its own engine carries the `Replicated` prefix, so
//! the family is rendered from the client's [`Topology`] (argument-less, the
//! server fills the Keeper path). It rejects `ON CLUSTER` on table DDL, so no
//! statement here carries one; `create-databases.sh` is the one site that
//! names a cluster, on `CREATE DATABASE`.
//!
//! That is also why this module exists rather than four `format!` calls spread
//! across two services: if the mechanism ever changes, every statement that
//! would have to change is here.
//!
//! Names and types are written as the caller gave them. This renders DDL, it
//! does not vet it: a name the warehouse will not take fails at the warehouse.
//!
//! Rendered through [`Client::ddl_sql`](crate::Client::ddl_sql), which knows
//! the topology and refuses a statement aimed at a database the connection
//! does not own.

use core::fmt::Write;

use crate::Topology;

/// The engine family, reachable only through [`TableSpec::merge_tree`].
const MERGE_TREE: &str = "MergeTree";
/// The prefix the family takes where the warehouse replicates.
const REPLICATED: &str = "Replicated";

/// What a create does about a name something already holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OnExisting {
    /// Fail. The name is this run's to mint, so anything holding it is a bug.
    #[default]
    Fail,
    /// Leave it standing: this create is repeating one that already finished.
    Keep,
}

/// The table a creator makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSpec {
    database: String,
    name: String,
    columns: Vec<(String, String)>,
    sort_key: Vec<String>,
    on_existing: OnExisting,
}

impl TableSpec {
    /// A `MergeTree`-family table of `columns`, sorted by `sort_key`. Whether
    /// the family is `ReplicatedMergeTree` is the rendering client's topology.
    ///
    /// `columns` are `(name, type)` in the order the DDL declares them, which
    /// is the physical column order the table ends up with.
    #[must_use]
    pub fn merge_tree(
        database: &str,
        name: &str,
        columns: &[(&str, &str)],
        sort_key: &[&str],
    ) -> Self {
        Self {
            database: database.to_owned(),
            name: name.to_owned(),
            columns: columns
                .iter()
                .map(|(name, data_type)| ((*name).to_owned(), (*data_type).to_owned()))
                .collect(),
            sort_key: sort_key.iter().map(|column| (*column).to_owned()).collect(),
            on_existing: OnExisting::Fail,
        }
    }

    /// `IF NOT EXISTS`: a name already held is this create repeating itself,
    /// not a collision.
    #[must_use]
    pub fn keep_existing(mut self) -> Self {
        self.on_existing = OnExisting::Keep;
        self
    }

    fn render(&self, topology: Topology) -> String {
        let mut sql = String::from("CREATE TABLE ");
        if self.on_existing == OnExisting::Keep {
            sql.push_str("IF NOT EXISTS ");
        }
        let _ = write!(sql, "{}", qualified(&self.database, &self.name));

        let block: Vec<String> = self
            .columns
            .iter()
            .map(|(name, data_type)| format!("    {name} {data_type}"))
            .collect();
        let _ = write!(sql, " (\n{}\n)", block.join(",\n"));

        let prefix = if topology.is_replicated() {
            REPLICATED
        } else {
            ""
        };
        let _ = write!(
            sql,
            " ENGINE = {prefix}{MERGE_TREE} ORDER BY {}",
            self.sort_key_sql()
        );

        sql
    }

    /// INVARIANT: bare. `system.tables.sorting_key` is how a reader recognises
    /// a table it made, and `ClickHouse` reads that key back unquoted.
    fn sort_key_sql(&self) -> String {
        match self.sort_key.as_slice() {
            [single] => single.clone(),
            several => format!("({})", several.join(", ")),
        }
    }
}

/// A statement that changes what the data layer holds.
///
/// Every variant names one database and acts inside it: a rename or an
/// exchange cannot cross databases through this API, and
/// [`Client::ddl_sql`](crate::Client::ddl_sql) refuses the one it is given if
/// that database is not the connection's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ddl {
    CreateTable(TableSpec),
    /// Always `IF EXISTS`: a drop racing a concurrent sweep, or a dataset
    /// already removed, has nothing left to do rather than something to report.
    DropTable {
        database: String,
        table: String,
    },
    /// Atomic on an Atomic database, which is ClickHouse's default — readers
    /// never observe a half-swapped pair.
    ExchangeTables {
        database: String,
        one: String,
        other: String,
    },
    RenameTable {
        database: String,
        from: String,
        to: String,
    },
}

impl Ddl {
    #[must_use]
    pub fn create_table(spec: TableSpec) -> Self {
        Self::CreateTable(spec)
    }

    #[must_use]
    pub fn drop_table(database: &str, table: &str) -> Self {
        Self::DropTable {
            database: database.to_owned(),
            table: table.to_owned(),
        }
    }

    #[must_use]
    pub fn exchange_tables(database: &str, one: &str, other: &str) -> Self {
        Self::ExchangeTables {
            database: database.to_owned(),
            one: one.to_owned(),
            other: other.to_owned(),
        }
    }

    #[must_use]
    pub fn rename_table(database: &str, from: &str, to: &str) -> Self {
        Self::RenameTable {
            database: database.to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
        }
    }

    /// The one database this statement acts in.
    pub(crate) fn database(&self) -> &str {
        match self {
            Self::CreateTable(spec) => &spec.database,
            Self::DropTable { database, .. }
            | Self::ExchangeTables { database, .. }
            | Self::RenameTable { database, .. } => database,
        }
    }

    pub(crate) fn render(&self, topology: Topology) -> String {
        match self {
            Self::CreateTable(spec) => spec.render(topology),
            Self::DropTable { database, table } => {
                format!("DROP TABLE IF EXISTS {}", qualified(database, table))
            }
            Self::ExchangeTables {
                database,
                one,
                other,
            } => format!(
                "EXCHANGE TABLES {} AND {}",
                qualified(database, one),
                qualified(database, other)
            ),
            Self::RenameTable { database, from, to } => format!(
                "RENAME TABLE {} TO {}",
                qualified(database, from),
                qualified(database, to)
            ),
        }
    }
}

/// Back-quoted because a dataset's name may carry a dash, which bare would
/// parse as a subtraction.
fn qualified(database: &str, table: &str) -> String {
    format!("`{database}`.`{table}`")
}

#[cfg(test)]
mod tests;
