use std::collections::BTreeMap;

use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait as _, DbBackend, FromQueryResult, Statement, TransactionTrait as _, Value,
};

use super::dashboards::{level_of, owner_of};
use super::visibility::{Clause, dashboard_unfiled, dashboard_visible, folder_visible};
use super::{MariaDefinitions, TotalRow};
use crate::domain::access::{Caller, Entry, FolderStanding, Level, Owner, PersonId};
use crate::domain::definition::{DefinitionId, DefinitionStoreError};
use crate::domain::folders::{
    Folder, FolderError, FolderId, FolderList, FolderName, FolderSummary, Folders, HeldFolder,
    MAX_FOLDERS, SeenFolder,
};

/// The folders the caller sees, each counting the dashboards in it they see.
const LIST_FOLDERS: &str = "SELECT folders.id, folders.name, folders.owner_person_id,
(SELECT COUNT(*) FROM folder_entries counted
JOIN dashboards ON dashboards.id = counted.dashboard
WHERE counted.folder_id = folders.id AND {visible}) AS dashboards
FROM folders
WHERE {folder_visible}
ORDER BY folders.name, folders.id";

const GRANTS_TO_CALLER: &str =
    "SELECT folder_id, level FROM folder_grants WHERE grantee IN (?, 'everyone')";

const COUNT_UNFILED: &str =
    "SELECT COUNT(*) AS total FROM dashboards WHERE {visible} AND {unfiled}";

const COUNT_FOLDERS: &str = "SELECT COUNT(*) AS total FROM folders WHERE owner_person_id = ?";

const INSERT_FOLDER: &str =
    "INSERT INTO folders (id, name, owner_person_id, created_at, updated_at)
VALUES (?, ?, ?, UTC_TIMESTAMP(6), UTC_TIMESTAMP(6))";

const FOLDER: &str = "SELECT id, name, owner_person_id FROM folders WHERE id = ?";

const FOLDER_GRANTS_REACHING: &str =
    "SELECT folder_id, level FROM folder_grants WHERE folder_id = ? AND grantee IN (?, 'everyone')";

const RENAME_FOLDER: &str =
    "UPDATE folders SET name = ?, updated_at = UTC_TIMESTAMP(6) WHERE id = ?";

const DELETE_FOLDER: &str = "DELETE FROM folders WHERE id = ?";

const FOLDERS_HOLDING: &str = "SELECT holding.id, holding.name, holding.owner_person_id
FROM folder_entries held
JOIN folders holding ON holding.id = held.folder_id
WHERE held.dashboard = ? AND {folder_visible}
ORDER BY holding.name, holding.id";

pub(super) const COUNT_DASHBOARD: &str = "SELECT COUNT(*) AS total FROM dashboards WHERE id = ?";

/// Places nothing unless the caller sees the dashboard as the row lands, so
/// an entry cannot rely on a grant withdrawn meanwhile. An entry already
/// there starts carrying, as this placer's, once a carrying one is asked for,
/// and never stops here.
///
/// INVARIANT: the driver reports found rows, so adding an entry already
/// there still counts one and only an unseen dashboard counts none.
const ADD_ENTRY: &str =
    "INSERT INTO folder_entries (folder_id, dashboard, placed_by, carries, created_at)
SELECT ?, dashboards.id, ?, ?, UTC_TIMESTAMP(6) FROM dashboards
WHERE dashboards.id = ? AND {visible}
ON DUPLICATE KEY UPDATE
placed_by = IF(? AND NOT folder_entries.carries, ?, folder_entries.placed_by),
carries = folder_entries.carries OR ?";

const REMOVE_ENTRY: &str = "DELETE FROM folder_entries WHERE folder_id = ? AND dashboard = ?";

#[derive(Debug, FromQueryResult)]
struct FolderRow {
    id: String,
    name: String,
    owner_person_id: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct SummaryRow {
    id: String,
    name: String,
    owner_person_id: Option<String>,
    dashboards: i64,
}

#[derive(Debug, FromQueryResult)]
struct FolderGrantRow {
    folder_id: String,
    level: String,
}

fn folder(id: &str, name: &str, owner: Option<&str>) -> Result<Folder, DefinitionStoreError> {
    let unreadable = || DefinitionStoreError::UnreadableRow("folder");

    Ok(Folder {
        id: FolderId::parse(id).map_err(|_| unreadable())?,
        name: FolderName::parse(name).map_err(|_| unreadable())?,
        owner: owner_of(owner)?,
    })
}

pub(super) fn statement(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::MySql, sql, values)
}

pub(super) fn list_statement(caller: Caller) -> Statement {
    let (visible, mut values) = dashboard_visible(caller);
    let (folders, folder_values) = folder_visible("folders", caller);
    values.extend(folder_values);

    let sql = LIST_FOLDERS
        .replace("{visible}", &visible)
        .replace("{folder_visible}", &folders);

    statement(&sql, values)
}

pub(super) fn unfiled_statement(caller: Caller) -> Statement {
    let (visible, mut values) = dashboard_visible(caller);
    let (unfiled, unfiled_values) = dashboard_unfiled(caller);
    values.extend(unfiled_values);

    let sql = COUNT_UNFILED
        .replace("{visible}", &visible)
        .replace("{unfiled}", &unfiled);

    statement(&sql, values)
}

pub(super) fn holding_statement(dashboard: &DefinitionId, caller: Caller) -> Statement {
    let (visible, visible_values) = folder_visible("holding", caller);
    let mut values: Vec<Value> = vec![dashboard.as_str().into()];
    values.extend(visible_values);

    statement(
        &FOLDERS_HOLDING.replace("{folder_visible}", &visible),
        values,
    )
}

pub(super) fn add_entry_statement(
    folder: FolderId,
    dashboard: &DefinitionId,
    caller: Caller,
    entry: Entry,
) -> Statement {
    let (visible, visible_values): Clause = dashboard_visible(caller);
    let placer: Value = caller.person.to_string().into();
    let carries: Value = entry.carries().into();
    let mut values: Vec<Value> = vec![
        folder.to_string().into(),
        placer.clone(),
        carries.clone(),
        dashboard.as_str().into(),
    ];
    values.extend(visible_values);
    values.extend([carries.clone(), placer, carries]);

    statement(&ADD_ENTRY.replace("{visible}", &visible), values)
}

/// The strongest grant level per folder among these rows.
fn strongest(rows: Vec<FolderGrantRow>) -> Result<BTreeMap<String, Level>, DefinitionStoreError> {
    let mut levels: BTreeMap<String, Level> = BTreeMap::new();

    for row in rows {
        let level = level_of(&row.level)?;
        let held = levels.entry(row.folder_id).or_insert(level);
        *held = (*held).max(level);
    }

    Ok(levels)
}

#[async_trait]
impl Folders for MariaDefinitions {
    async fn list_folders(&self, caller: Caller) -> Result<FolderList, FolderError> {
        let rows = SummaryRow::find_by_statement(list_statement(caller))
            .all(&self.db)
            .await?;
        let grants = FolderGrantRow::find_by_statement(statement(
            GRANTS_TO_CALLER,
            vec![caller.person.to_string().into()],
        ))
        .all(&self.db)
        .await?;
        let unfiled = TotalRow::find_by_statement(unfiled_statement(caller))
            .one(&self.db)
            .await?
            .map_or(0, |row| row.total);

        let levels = strongest(grants)?;
        let mut folders = Vec::with_capacity(rows.len());
        for row in rows {
            let held = HeldFolder {
                standing: FolderStanding {
                    owner: owner_of(row.owner_person_id.as_deref())?,
                    granted: levels.get(&row.id).copied(),
                },
                folder: folder(&row.id, &row.name, row.owner_person_id.as_deref())?,
            };
            let Some(seen) = SeenFolder::of(caller, held) else {
                continue;
            };
            folders.push(FolderSummary {
                seen,
                dashboards: u64::try_from(row.dashboards).unwrap_or(0),
            });
        }

        Ok(FolderList {
            folders,
            unfiled: u64::try_from(unfiled).unwrap_or(0),
        })
    }

    async fn create_folder(
        &self,
        owner: PersonId,
        name: FolderName,
    ) -> Result<Folder, FolderError> {
        let transaction = self.db.begin().await?;
        let held =
            TotalRow::find_by_statement(statement(COUNT_FOLDERS, vec![owner.to_string().into()]))
                .one(&transaction)
                .await?
                .map_or(0, |row| row.total);
        if usize::try_from(held).unwrap_or(usize::MAX) >= MAX_FOLDERS {
            return Err(FolderError::TooMany);
        }

        let id = FolderId::new();
        transaction
            .execute_raw(statement(
                INSERT_FOLDER,
                vec![
                    id.to_string().into(),
                    name.as_str().into(),
                    owner.to_string().into(),
                ],
            ))
            .await?;
        transaction.commit().await?;

        Ok(Folder {
            id,
            name,
            owner: Owner::Person(owner),
        })
    }

    async fn held_folder(
        &self,
        id: FolderId,
        caller: Caller,
    ) -> Result<Option<HeldFolder>, DefinitionStoreError> {
        let row = FolderRow::find_by_statement(statement(FOLDER, vec![id.to_string().into()]))
            .one(&self.db)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let grants = FolderGrantRow::find_by_statement(statement(
            FOLDER_GRANTS_REACHING,
            vec![id.to_string().into(), caller.person.to_string().into()],
        ))
        .all(&self.db)
        .await?;

        let held = folder(&row.id, &row.name, row.owner_person_id.as_deref())?;
        let granted = strongest(grants)?.into_values().max();

        Ok(Some(HeldFolder {
            standing: FolderStanding {
                owner: held.owner,
                granted,
            },
            folder: held,
        }))
    }

    async fn rename_folder(&self, id: FolderId, name: &FolderName) -> Result<bool, FolderError> {
        let result = self
            .db
            .execute_raw(statement(
                RENAME_FOLDER,
                vec![name.as_str().into(), id.to_string().into()],
            ))
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn delete_folder(&self, id: FolderId) -> Result<bool, FolderError> {
        let result = self
            .db
            .execute_raw(statement(DELETE_FOLDER, vec![id.to_string().into()]))
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn folders_holding(
        &self,
        dashboard: &DefinitionId,
        caller: Caller,
    ) -> Result<Vec<Folder>, FolderError> {
        let rows = FolderRow::find_by_statement(holding_statement(dashboard, caller))
            .all(&self.db)
            .await?;

        let mut folders = Vec::with_capacity(rows.len());
        for row in rows {
            folders.push(folder(&row.id, &row.name, row.owner_person_id.as_deref())?);
        }

        Ok(folders)
    }

    async fn add_entry(
        &self,
        folder: FolderId,
        dashboard: &DefinitionId,
        caller: Caller,
        entry: Entry,
    ) -> Result<(), FolderError> {
        let added = self
            .db
            .execute_raw(add_entry_statement(folder, dashboard, caller, entry))
            .await
            .map_err(|error| match error.sql_err() {
                Some(sea_orm::SqlErr::ForeignKeyConstraintViolation(_)) => {
                    FolderError::FolderNotFound(folder)
                }
                _ => error.into(),
            })?;
        if added.rows_affected() == 0 {
            return Err(FolderError::DashboardNotFound(
                dashboard.as_str().to_owned(),
            ));
        }

        Ok(())
    }

    async fn remove_entry(
        &self,
        folder: FolderId,
        dashboard: &DefinitionId,
    ) -> Result<bool, FolderError> {
        let removed = self
            .db
            .execute_raw(statement(
                REMOVE_ENTRY,
                vec![folder.to_string().into(), dashboard.as_str().into()],
            ))
            .await?;

        Ok(removed.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests;
