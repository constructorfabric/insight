//! Metric, widget and dashboard definitions.
//!
//! These live in `MariaDB`, not in `ClickHouse` with the data they describe.
//! They are a few hundred rows read by name and edited in place by whoever is
//! asking the assistant for a change — which is what a row store is for, and
//! what a column store is not. On `ClickHouse` a change meant inserting a new
//! version and reading with `FINAL`, nothing stopped two rows claiming one
//! name, and a create that failed halfway left what it had already written
//! behind with nothing to roll it back with.

pub(crate) mod migration;

use std::fmt;

use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait as _, DatabaseConnection, DbBackend, FromQueryResult, Statement,
    TransactionTrait as _,
};
use thiserror::Error;

const MAX_NAME_CHARS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DefinitionKind {
    Metric,
    Widget,
    Dashboard,
}

impl DefinitionKind {
    pub(crate) fn table(self) -> &'static str {
        match self {
            Self::Metric => "metrics",
            Self::Widget => "widgets",
            Self::Dashboard => "dashboards",
        }
    }

    pub(crate) fn singular(self) -> &'static str {
        match self {
            Self::Metric => "metric",
            Self::Widget => "widget",
            Self::Dashboard => "dashboard",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionName(String);

impl DefinitionName {
    pub(crate) fn parse(value: &str) -> Result<Self, DefinitionError> {
        if value.is_empty() || value.chars().count() > MAX_NAME_CHARS {
            return Err(DefinitionError::Name);
        }

        if !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(DefinitionError::Name);
        }

        Ok(Self(value.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The largest page a caller may ask for, and what it gets by default.
pub(crate) const DEFAULT_PAGE_LIMIT: u64 = 50;
pub(crate) const MAX_PAGE_LIMIT: u64 = 200;

/// How much of a catalogue to read.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Page {
    limit: u64,
    offset: u64,
}

impl Page {
    pub(crate) fn parse(limit: Option<u64>, offset: Option<u64>) -> Result<Self, PageError> {
        let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT);
        if limit == 0 || limit > MAX_PAGE_LIMIT {
            return Err(PageError::Limit(MAX_PAGE_LIMIT));
        }

        Ok(Self {
            limit,
            offset: offset.unwrap_or(0),
        })
    }

    pub(crate) fn limit(self) -> u64 {
        self.limit
    }

    pub(crate) fn offset(self) -> u64 {
        self.offset
    }
}

/// One page of a catalogue, and how many definitions it is a page of.
#[derive(Debug)]
pub(crate) struct NamePage {
    pub(crate) names: Vec<String>,
    /// Every match, not the page — so a reader can say what is behind it.
    pub(crate) total: u64,
}

/// One write in a batch.
///
/// A rename is a put under the new name, a delete of the old one, and a put
/// per dependent that pointed at it - which is only a rename if all of them
/// land together.
#[derive(Debug)]
pub(crate) enum Change {
    Put(DefinitionKind, DefinitionName, serde_json::Value),
    Delete(DefinitionKind, DefinitionName),
}

/// What the API and the chat need of the store, so neither has to know where
/// definitions live — and so their tests can hold them in a map rather than
/// answer a database's wire protocol.
#[async_trait]
pub(crate) trait Definitions: Send + Sync + fmt::Debug {
    /// Stores `body` under `name`, replacing whatever that name held.
    async fn put(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError>;

    async fn get(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Option<serde_json::Value>, DefinitionStoreError>;

    /// Every name of this kind.
    ///
    /// For the dependency scans, which have to see the definitions a page
    /// would leave out.
    async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, DefinitionStoreError>;

    /// One page of the names of this kind whose name or body holds `needle`.
    ///
    /// The body too, because "which metrics read `class_git_commits`" is the
    /// question a catalogue of a few hundred definitions is actually asked,
    /// and a name cannot answer it. An empty needle is every definition of
    /// that kind.
    async fn page(
        &self,
        kind: DefinitionKind,
        needle: &str,
        page: Page,
    ) -> Result<NamePage, DefinitionStoreError>;

    /// Removes the definition, reporting whether there was one.
    async fn delete(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<bool, DefinitionStoreError>;

    /// Applies every change or none of them.
    ///
    /// A chat request builds a metric, its widgets and the dashboard that
    /// holds them; written one at a time, a failure partway through left the
    /// reader a metric, no dashboard, and no way to tell.
    async fn apply(&self, changes: &[Change]) -> Result<(), DefinitionStoreError>;
}

/// The name is the primary key, so a write is an upsert and two writers cannot
/// leave two rows claiming one name.
const UPSERT: &str = "INSERT INTO {table} (name, body, updated_at)
VALUES (?, ?, UTC_TIMESTAMP(6))
ON DUPLICATE KEY UPDATE body = VALUES(body), updated_at = VALUES(updated_at)";

const DELETE_ONE: &str = "DELETE FROM {table} WHERE name = ?";
const SELECT_BODY: &str = "SELECT body FROM {table} WHERE name = ?";
const SELECT_NAMES: &str = "SELECT name FROM {table} ORDER BY name";

/// No ESCAPE clause: backslash is already the default LIKE escape here, and
/// spelling it out needs a lone backslash in a string literal, which MariaDB
/// reads as escaping the closing quote (error 1064).
const PAGE_NAMES: &str = "SELECT name FROM {table}
WHERE name LIKE ? OR body LIKE ?
ORDER BY name
LIMIT ? OFFSET ?";

const COUNT_MATCHES: &str = "SELECT COUNT(*) AS total FROM {table}
WHERE name LIKE ? OR body LIKE ?";

/// `{table}` is substituted from [`DefinitionKind::table`], which returns one
/// of three literals — never anything a request carries. Every value is bound.
fn sql(template: &str, kind: DefinitionKind) -> String {
    template.replace("{table}", kind.table())
}

/// A needle as a literal inside `LIKE`.
///
/// `%` and `_` are wildcards there, so a search for `pr_merged` would match
/// `prXmerged` — the caller typed a name, not a pattern.
fn like_escaped(needle: &str) -> String {
    needle
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[derive(Debug, FromQueryResult)]
struct BodyRow {
    body: String,
}

#[derive(Debug, FromQueryResult)]
struct NameRow {
    name: String,
}

#[derive(Debug, FromQueryResult)]
struct TotalRow {
    total: i64,
}

pub(crate) struct MariaDefinitions {
    db: DatabaseConnection,
}

impl MariaDefinitions {
    pub(crate) fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    fn upsert(
        kind: DefinitionKind,
        name: &DefinitionName,
        body: &serde_json::Value,
    ) -> Result<Statement, DefinitionStoreError> {
        Ok(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql(UPSERT, kind),
            [name.as_str().into(), serde_json::to_string(body)?.into()],
        ))
    }
}

#[async_trait]
impl Definitions for MariaDefinitions {
    async fn put(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError> {
        self.db.execute_raw(Self::upsert(kind, name, body)?).await?;
        Ok(())
    }

    async fn get(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Option<serde_json::Value>, DefinitionStoreError> {
        let row = BodyRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql(SELECT_BODY, kind),
            [name.as_str().into()],
        ))
        .one(&self.db)
        .await?;

        match row {
            Some(row) => Ok(Some(serde_json::from_str(&row.body)?)),
            None => Ok(None),
        }
    }

    async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, DefinitionStoreError> {
        let rows = NameRow::find_by_statement(Statement::from_string(
            DbBackend::MySql,
            sql(SELECT_NAMES, kind),
        ))
        .all(&self.db)
        .await?;

        Ok(rows.into_iter().map(|row| row.name).collect())
    }

    async fn page(
        &self,
        kind: DefinitionKind,
        needle: &str,
        page: Page,
    ) -> Result<NamePage, DefinitionStoreError> {
        let pattern = format!("%{}%", like_escaped(needle));
        let rows = NameRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql(PAGE_NAMES, kind),
            [
                pattern.clone().into(),
                pattern.clone().into(),
                page.limit.into(),
                page.offset.into(),
            ],
        ))
        .all(&self.db)
        .await?;

        let counted = TotalRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql(COUNT_MATCHES, kind),
            [pattern.clone().into(), pattern.into()],
        ))
        .one(&self.db)
        .await?;

        Ok(NamePage {
            names: rows.into_iter().map(|row| row.name).collect(),
            total: counted.map_or(0, |row| u64::try_from(row.total).unwrap_or(0)),
        })
    }

    async fn delete(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<bool, DefinitionStoreError> {
        let result = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                sql(DELETE_ONE, kind),
                [name.as_str().into()],
            ))
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn apply(&self, changes: &[Change]) -> Result<(), DefinitionStoreError> {
        let transaction = self.db.begin().await?;
        for change in changes {
            let statement = match change {
                Change::Put(kind, name, body) => Self::upsert(*kind, name, body)?,
                Change::Delete(kind, name) => Statement::from_sql_and_values(
                    DbBackend::MySql,
                    sql(DELETE_ONE, *kind),
                    [name.as_str().into()],
                ),
            };
            transaction.execute_raw(statement).await?;
        }
        transaction.commit().await?;

        Ok(())
    }
}

impl fmt::Debug for MariaDefinitions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MariaDefinitions")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Error)]
pub(crate) enum DefinitionError {
    #[error("definition names use letters, digits, underscore and dash, up to 128 characters")]
    Name,
}

#[derive(Clone, Copy, Debug, Error)]
pub(crate) enum PageError {
    #[error("limit must be between 1 and {0}")]
    Limit(u64),
}

#[derive(Debug, Error)]
pub(crate) enum DefinitionStoreError {
    #[error("definition store operation failed")]
    Database(#[from] sea_orm::DbErr),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
pub(crate) mod memory;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn names_reject_anything_outside_the_identifier_charset() {
        assert!(DefinitionName::parse("commits_per_day").is_ok());
        assert!(DefinitionName::parse("").is_err());
        assert!(DefinitionName::parse("drop table").is_err());
        assert!(DefinitionName::parse("a`b").is_err());
        assert!(DefinitionName::parse(&"a".repeat(129)).is_err());
    }

    #[test]
    fn each_kind_has_its_own_table() {
        assert_eq!(DefinitionKind::Metric.table(), "metrics");
        assert_eq!(DefinitionKind::Widget.table(), "widgets");
        assert_eq!(DefinitionKind::Dashboard.table(), "dashboards");
    }

    #[test]
    fn a_write_upserts_on_the_name_so_a_change_is_one_statement() {
        let statement = sql(UPSERT, DefinitionKind::Dashboard);

        assert!(statement.contains("INSERT INTO dashboards"), "{statement}");
        assert!(
            statement.contains("ON DUPLICATE KEY UPDATE body = VALUES(body)"),
            "{statement}"
        );
    }

    #[test]
    fn a_delete_is_by_name_in_the_kind_table() {
        assert_eq!(
            sql(DELETE_ONE, DefinitionKind::Dashboard),
            "DELETE FROM dashboards WHERE name = ?"
        );
    }

    #[test]
    fn reads_are_by_name_and_never_interpolate_it() {
        let one = sql(SELECT_BODY, DefinitionKind::Metric);
        let all = sql(SELECT_NAMES, DefinitionKind::Widget);

        assert_eq!(one, "SELECT body FROM metrics WHERE name = ?");
        assert_eq!(all, "SELECT name FROM widgets ORDER BY name");
    }

    #[test]
    fn the_bound_values_are_the_name_and_the_body() {
        let name = DefinitionName::parse("commits_per_day")
            .unwrap_or_else(|error| panic!("name must parse: {error}"));

        let statement =
            MariaDefinitions::upsert(DefinitionKind::Metric, &name, &json!({ "table": "events" }))
                .unwrap_or_else(|error| panic!("the statement builds: {error}"));

        let values = statement
            .values
            .as_ref()
            .map(|values| values.0.clone())
            .unwrap_or_default();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].to_string(), "'commits_per_day'");
        assert!(values[1].to_string().contains("events"), "{:?}", values[1]);
    }
}
