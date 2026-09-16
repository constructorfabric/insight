//! The definitions as `MariaDB` holds them.

use std::fmt;

use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait as _, DatabaseConnection, DbBackend, FromQueryResult, Statement,
    TransactionTrait as _,
};

use super::{
    Change, DefinitionKind, DefinitionName, DefinitionStoreError, Definitions, NamePage, Page,
};

/// The name is the primary key, so a write is an upsert and two writers cannot
/// leave two rows claiming one name.
const UPSERT: &str = "INSERT INTO {table} (name, body, updated_at)
VALUES (?, ?, UTC_TIMESTAMP(6))
ON DUPLICATE KEY UPDATE body = VALUES(body), updated_at = VALUES(updated_at)";

const INSERT_NEW: &str = "INSERT INTO {table} (name, body, updated_at)
VALUES (?, ?, UTC_TIMESTAMP(6))";

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
        Self::statement_for(UPSERT, kind, name, body)
    }

    fn statement_for(
        template: &str,
        kind: DefinitionKind,
        name: &DefinitionName,
        body: &serde_json::Value,
    ) -> Result<Statement, DefinitionStoreError> {
        Ok(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql(template, kind),
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
                Change::Create(kind, name, body) => {
                    Self::statement_for(INSERT_NEW, *kind, name, body)?
                }
                Change::Delete(kind, name) => Statement::from_sql_and_values(
                    DbBackend::MySql,
                    sql(DELETE_ONE, *kind),
                    [name.as_str().into()],
                ),
            };
            transaction
                .execute_raw(statement)
                .await
                .map_err(|error| taken_or(error, change))?;
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

/// A write that was to create a name someone already holds, told apart from
/// every other database failure so the caller hears which it was.
fn taken_or(error: sea_orm::DbErr, change: &Change) -> DefinitionStoreError {
    let Change::Create(_, name, _) = change else {
        return DefinitionStoreError::Database(error);
    };

    match error.sql_err() {
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_)) => {
            DefinitionStoreError::NameTaken(name.as_str().to_owned())
        }

        _ => DefinitionStoreError::Database(error),
    }
}

#[cfg(test)]
mod tests;
