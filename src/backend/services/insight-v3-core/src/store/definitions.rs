//! The definitions as `MariaDB` holds them.

mod dashboards;
mod folders;
pub(crate) mod migration;
mod pins;
mod sharing;
mod tags;
mod visibility;

#[cfg(test)]
pub(crate) mod memory;

use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use chrono::NaiveDateTime;
use sea_orm::{
    ConnectionTrait as _, DatabaseConnection, DatabaseTransaction, DbBackend, FromQueryResult,
    Statement, TransactionTrait as _,
};

use crate::domain::access::Caller;
use crate::domain::definition::{
    Change, DefinitionId, DefinitionKind, DefinitionLabel, DefinitionStoreError, Definitions,
    LibraryEntry, LibraryKind, LibraryPage, Listed, ListedPage, Lookup, Page, StoredDefinition,
};
use crate::store::like_escaped;

/// The id is the primary key, so a create that meets one already held fails
/// rather than overwriting it.
const INSERT_NEW: &str = "INSERT INTO {table} (id, name, body, updated_at)
VALUES (?, ?, ?, UTC_TIMESTAMP(6))";

const REPLACE_ONE: &str =
    "UPDATE {table} SET name = ?, body = ?, updated_at = UTC_TIMESTAMP(6) WHERE id = ?";

const DELETE_ONE: &str = "DELETE FROM {table} WHERE id = ?";

const OWN: &str = "UPDATE dashboards SET owner_person_id = ? WHERE id = ?";

/// Each owned folder holding the original that the placer may arrange: their
/// own, or one granted to them or to everyone as editor while they are an
/// administrator.
const CARRY_ENTRIES: &str =
    "INSERT INTO folder_entries (folder_id, dashboard, placed_by, carries, created_at)
SELECT kept.folder_id, ?, ?, TRUE, UTC_TIMESTAMP(6) FROM folder_entries kept
JOIN folders kept_folder ON kept_folder.id = kept.folder_id
WHERE kept.dashboard = ? AND kept_folder.owner_person_id IS NOT NULL
AND (kept_folder.owner_person_id = ? OR ({admin} AND EXISTS (SELECT 1 FROM folder_grants kept_grants
WHERE kept_grants.folder_id = kept_folder.id AND kept_grants.grantee IN (?, 'everyone')
AND kept_grants.level = 'editor')))";

const SELECT_BODY: &str = "SELECT body FROM {table} WHERE id = ?";
const SELECT_ONE: &str = "SELECT name, body, updated_at FROM {table} WHERE id = ?";
const SELECT_IDS: &str = "SELECT id FROM {table} ORDER BY id";

/// No ESCAPE clause: backslash is already the default LIKE escape here, and
/// spelling it out needs a lone backslash in a string literal, which MariaDB
/// reads as escaping the closing quote (error 1064).
// INVARIANT: one search and order for every page of a library, so each reader and `COUNT_MATCHES` agree on what a page holds.
const PAGE: &str = "SELECT {columns} FROM {table}
WHERE name LIKE ? OR body LIKE ?
ORDER BY name, id
LIMIT ? OFFSET ?";

const LISTED_COLUMNS: &str = "id, name";
const ENTRY_COLUMNS: &str = "id, name, body, updated_at";

const NAMES_OF: &str = "SELECT id, name FROM {table} WHERE id IN ({ids})";

const COUNT_MATCHES: &str = "SELECT COUNT(*) AS total FROM {table}
WHERE name LIKE ? OR body LIKE ?";

/// `{table}` is substituted from [`DefinitionKind::table`], which returns one
/// of three literals — never anything a request carries. Every value is bound.
fn sql(template: &str, kind: DefinitionKind) -> String {
    template.replace("{table}", kind.table())
}

/// One read of the names of `ids`, each bound rather than written into it.
fn names_statement(kind: LibraryKind, ids: &[&str]) -> Statement {
    let marks = vec!["?"; ids.len()].join(", ");
    let template = NAMES_OF.replace("{ids}", &marks);
    let values: Vec<sea_orm::Value> = ids.iter().map(|id| (*id).into()).collect();

    Statement::from_sql_and_values(DbBackend::MySql, sql(&template, kind.kind()), values)
}

fn like_pattern(needle: &str) -> String {
    format!("%{}%", like_escaped(needle))
}

/// One page of `template`'s rows matching `pattern` in a name or a body.
fn page_sql(columns: &str, kind: DefinitionKind) -> String {
    sql(&PAGE.replace("{columns}", columns), kind)
}

fn page_statement(columns: &str, kind: LibraryKind, pattern: &str, page: Page) -> Statement {
    Statement::from_sql_and_values(
        DbBackend::MySql,
        page_sql(columns, kind.kind()),
        [
            pattern.into(),
            pattern.into(),
            page.limit().into(),
            page.offset().into(),
        ],
    )
}

#[derive(Debug, FromQueryResult)]
struct BodyRow {
    body: String,
}

#[derive(Debug, FromQueryResult)]
struct StoredRow {
    name: String,
    body: String,
    updated_at: NaiveDateTime,
}

#[derive(Debug, FromQueryResult)]
struct IdRow {
    id: String,
}

#[derive(Debug, FromQueryResult)]
struct ListedRow {
    id: String,
    name: String,
}

impl ListedRow {
    fn into_listed(self) -> Listed {
        Listed {
            id: self.id,
            name: self.name,
        }
    }
}

#[derive(Debug, FromQueryResult)]
struct EntryRow {
    id: String,
    name: String,
    body: String,
    updated_at: NaiveDateTime,
}

impl EntryRow {
    fn into_entry(self) -> Result<LibraryEntry, DefinitionStoreError> {
        Ok(LibraryEntry {
            id: self.id,
            name: self.name,
            body: serde_json::from_str(&self.body)?,
            updated_at: self.updated_at.and_utc(),
        })
    }
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

    /// The connection itself, for a live test seeding rows no store method
    /// writes.
    #[cfg(test)]
    pub(crate) fn connection(&self) -> &DatabaseConnection {
        &self.db
    }

    fn insert_statement(
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> Result<Statement, DefinitionStoreError> {
        Ok(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql(INSERT_NEW, kind),
            [
                id.as_str().into(),
                name.as_str().into(),
                serde_json::to_string(body)?.into(),
            ],
        ))
    }

    fn replace_statement(
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> Result<Statement, DefinitionStoreError> {
        Ok(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql(REPLACE_ONE, kind),
            [
                name.as_str().into(),
                serde_json::to_string(body)?.into(),
                id.as_str().into(),
            ],
        ))
    }

    /// One change of a batch, inside its transaction.
    async fn apply_one(
        transaction: &DatabaseTransaction,
        change: &Change,
    ) -> Result<(), DefinitionStoreError> {
        let statement = match change {
            Change::Create(kind, id, name, body) => Self::insert_statement(*kind, id, name, body)?,
            Change::Replace(kind, id, name, body) => {
                Self::replace_statement(*kind, id, name, body)?
            }
            Change::Own { dashboard, owner } => Statement::from_sql_and_values(
                DbBackend::MySql,
                OWN,
                [owner.to_string().into(), dashboard.as_str().into()],
            ),
            Change::CarryEntries { from, to, placer } => carry_entries(from, to, *placer),
            Change::CarryTags { from, to, scope } => {
                return tags::carry_tags(transaction, from, to, *scope).await;
            }
        };

        let result = transaction
            .execute_raw(statement)
            .await
            .map_err(|error| refused_or(error, change))?;

        if let Change::Replace(_, id, _, _) = change
            && result.rows_affected() == 0
        {
            return Err(DefinitionStoreError::Missing(id.as_str().to_owned()));
        }

        Ok(())
    }
}

#[async_trait]
impl Lookup for MariaDefinitions {
    async fn get(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<Option<serde_json::Value>, DefinitionStoreError> {
        let row = BodyRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql(SELECT_BODY, kind),
            [id.as_str().into()],
        ))
        .one(&self.db)
        .await?;

        match row {
            Some(row) => Ok(Some(serde_json::from_str(&row.body)?)),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl Definitions for MariaDefinitions {
    async fn read(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<Option<StoredDefinition>, DefinitionStoreError> {
        let row = StoredRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql(SELECT_ONE, kind),
            [id.as_str().into()],
        ))
        .one(&self.db)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(StoredDefinition {
            name: row.name,
            body: serde_json::from_str(&row.body)?,
            updated_at: row.updated_at.and_utc(),
        }))
    }

    #[cfg(test)]
    async fn create(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError> {
        let statement = Self::insert_statement(kind, id, name, body)?;
        self.db
            .execute_raw(statement)
            .await
            .map_err(|error| taken_or(error, id))?;

        Ok(())
    }

    async fn replace(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> Result<bool, DefinitionStoreError> {
        let statement = Self::replace_statement(kind, id, name, body)?;
        let result = self.db.execute_raw(statement).await?;

        Ok(result.rows_affected() > 0)
    }

    async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, DefinitionStoreError> {
        let rows = IdRow::find_by_statement(Statement::from_string(
            DbBackend::MySql,
            sql(SELECT_IDS, kind),
        ))
        .all(&self.db)
        .await?;

        Ok(rows.into_iter().map(|row| row.id).collect())
    }

    async fn page(
        &self,
        kind: LibraryKind,
        needle: &str,
        page: Page,
    ) -> Result<ListedPage, DefinitionStoreError> {
        let pattern = like_pattern(needle);
        let rows =
            ListedRow::find_by_statement(page_statement(LISTED_COLUMNS, kind, &pattern, page))
                .all(&self.db)
                .await?;

        Ok(ListedPage {
            items: rows.into_iter().map(ListedRow::into_listed).collect(),
            total: self.count_matches(kind, pattern).await?,
        })
    }

    async fn page_entries(
        &self,
        kind: LibraryKind,
        needle: &str,
        page: Page,
    ) -> Result<LibraryPage, DefinitionStoreError> {
        let pattern = like_pattern(needle);
        let rows = EntryRow::find_by_statement(page_statement(ENTRY_COLUMNS, kind, &pattern, page))
            .all(&self.db)
            .await?;
        let items = rows
            .into_iter()
            .map(EntryRow::into_entry)
            .collect::<Result<_, _>>()?;

        Ok(LibraryPage {
            items,
            total: self.count_matches(kind, pattern).await?,
        })
    }

    async fn names_of(
        &self,
        kind: LibraryKind,
        ids: &[&str],
    ) -> Result<HashMap<String, String>, DefinitionStoreError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows = ListedRow::find_by_statement(names_statement(kind, ids))
            .all(&self.db)
            .await?;

        Ok(rows.into_iter().map(|row| (row.id, row.name)).collect())
    }

    async fn delete(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<bool, DefinitionStoreError> {
        let removal = Statement::from_sql_and_values(
            DbBackend::MySql,
            sql(DELETE_ONE, kind),
            [id.as_str().into()],
        );
        if kind == DefinitionKind::Dashboard {
            return self.delete_dashboard(removal).await;
        }

        let result = self.db.execute_raw(removal).await?;

        Ok(result.rows_affected() > 0)
    }

    async fn apply(&self, changes: &[Change]) -> Result<(), DefinitionStoreError> {
        let transaction = self.db.begin().await?;
        if changes
            .iter()
            .any(|change| matches!(change, Change::CarryTags { .. }))
        {
            tags::lock_tags(&transaction).await?;
        }

        for change in changes {
            Self::apply_one(&transaction, change).await?;
        }
        transaction.commit().await?;

        Ok(())
    }
}

impl MariaDefinitions {
    async fn count_matches(
        &self,
        kind: LibraryKind,
        pattern: String,
    ) -> Result<u64, DefinitionStoreError> {
        let counted = TotalRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql(COUNT_MATCHES, kind.kind()),
            [pattern.clone().into(), pattern.into()],
        ))
        .one(&self.db)
        .await?;

        Ok(counted.map_or(0, |row| u64::try_from(row.total).unwrap_or(0)))
    }
}

impl fmt::Debug for MariaDefinitions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MariaDefinitions")
            .finish_non_exhaustive()
    }
}

/// A create that met an id someone already holds, told apart from every
/// other database failure so the caller hears which it was.
fn taken_or(error: sea_orm::DbErr, id: &DefinitionId) -> DefinitionStoreError {
    match error.sql_err() {
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_)) => {
            DefinitionStoreError::IdTaken(id.as_str().to_owned())
        }

        _ => DefinitionStoreError::Database(error),
    }
}

pub(super) fn carry_entries(from: &DefinitionId, to: &DefinitionId, placer: Caller) -> Statement {
    let person = placer.person.to_string();
    let sql = CARRY_ENTRIES.replace("{admin}", visibility::administrator_literal(placer));

    Statement::from_sql_and_values(
        DbBackend::MySql,
        sql,
        [
            to.as_str().into(),
            person.clone().into(),
            from.as_str().into(),
            person.clone().into(),
            person.into(),
        ],
    )
}

fn refused_or(error: sea_orm::DbErr, change: &Change) -> DefinitionStoreError {
    match change {
        Change::Create(_, id, _, _) => taken_or(error, id),
        Change::Replace(..)
        | Change::Own { .. }
        | Change::CarryEntries { .. }
        | Change::CarryTags { .. } => DefinitionStoreError::Database(error),
    }
}

#[cfg(test)]
mod tests;
