use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait as _, DbBackend, FromQueryResult, Statement, TransactionTrait as _, Value,
};

use super::dashboards::{IN_FOLDER, UNFILED, VISIBLE, owner_of, visible_values};
use super::{MariaDefinitions, TotalRow};
use crate::domain::access::{Owner, PersonId};
use crate::domain::definition::DefinitionId;
use crate::domain::folders::{
    Folder, FolderError, FolderId, FolderList, FolderName, FolderSummary, Folders, MAX_FOLDERS,
};

/// The caller's folders and the ownerless ones, each counting the dashboards
/// in it the caller sees.
const LIST_FOLDERS: &str = "SELECT folders.id, folders.name, folders.owner_person_id,
(SELECT COUNT(*) FROM dashboards WHERE {visible} AND {in_folder}) AS dashboards
FROM folders
WHERE folders.owner_person_id IS NULL OR folders.owner_person_id = ?
ORDER BY folders.name, folders.id";

const COUNT_UNFILED: &str =
    "SELECT COUNT(*) AS total FROM dashboards WHERE {visible} AND {unfiled}";

const COUNT_FOLDERS: &str = "SELECT COUNT(*) AS total FROM folders WHERE owner_person_id = ?";

const INSERT_FOLDER: &str =
    "INSERT INTO folders (id, name, owner_person_id, created_at, updated_at)
VALUES (?, ?, ?, UTC_TIMESTAMP(6), UTC_TIMESTAMP(6))";

/// A folder the caller reaches: their own, or an ownerless one.
const REACHABLE: &str = "(owner_person_id IS NULL OR owner_person_id = ?)";

const RENAME_FOLDER: &str =
    "UPDATE folders SET name = ?, updated_at = UTC_TIMESTAMP(6) WHERE id = ? AND {reachable}";

const DELETE_FOLDER: &str = "DELETE FROM folders WHERE id = ? AND {reachable}";

const FOLDER_OWNER: &str = "SELECT owner_person_id FROM folders WHERE id = ?";

const FOLDER_OF: &str = "SELECT folders.id, folders.name, folders.owner_person_id
FROM dashboards JOIN folders ON folders.id = dashboards.folder_id
WHERE dashboards.id = ? AND dashboards.owner_person_id IS NULL
UNION ALL
SELECT folders.id, folders.name, folders.owner_person_id
FROM dashboards
JOIN dashboard_placements ON dashboard_placements.dashboard = dashboards.id
JOIN folders ON folders.id = dashboard_placements.folder_id
WHERE dashboards.id = ? AND dashboards.owner_person_id IS NOT NULL
AND dashboard_placements.person = ?";

pub(super) const COUNT_DASHBOARD: &str = "SELECT COUNT(*) AS total FROM dashboards WHERE id = ?";

const FILE_DASHBOARD: &str = "UPDATE dashboards SET folder_id = ? WHERE id = ?";

/// Places nothing unless the person sees the dashboard as the row lands, so a
/// placement cannot outlive the share it relied on being withdrawn meanwhile.
///
/// INVARIANT: the driver reports found rows, so re-filing into the same
/// folder still counts one and only an unseen dashboard counts none.
const PLACE: &str = "INSERT INTO dashboard_placements (person, dashboard, folder_id)
SELECT ?, dashboards.id, ? FROM dashboards
WHERE dashboards.id = ? AND {visible}
ON DUPLICATE KEY UPDATE dashboard_placements.folder_id = ?";

const UNPLACE: &str = "DELETE FROM dashboard_placements WHERE person = ? AND dashboard = ?";

#[derive(Debug, FromQueryResult)]
struct FolderRow {
    id: String,
    name: String,
    owner_person_id: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct OwnerRow {
    owner_person_id: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct SummaryRow {
    id: String,
    name: String,
    owner_person_id: Option<String>,
    dashboards: i64,
}

fn folder(id: &str, name: &str, owner: Option<&str>) -> Result<Folder, FolderError> {
    Ok(Folder {
        id: FolderId::parse(id)?,
        name: FolderName::parse(name)?,
        owner: owner_of(owner)?,
    })
}

pub(super) fn statement(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::MySql, sql, values)
}

pub(super) fn list_statement(caller: PersonId) -> Statement {
    let sql = LIST_FOLDERS
        .replace("{visible}", VISIBLE)
        .replace("{in_folder}", &IN_FOLDER.replace("{folder}", "folders.id"));
    let mut values = visible_values(caller);
    values.push(caller.to_string().into());
    values.push(caller.to_string().into());

    statement(&sql, values)
}

pub(super) fn unfiled_statement(caller: PersonId) -> Statement {
    let sql = COUNT_UNFILED
        .replace("{visible}", VISIBLE)
        .replace("{unfiled}", UNFILED);
    let mut values = visible_values(caller);
    values.push(caller.to_string().into());

    statement(&sql, values)
}

/// The write filing a dashboard among `folders_of`.
pub(super) fn file_statement(
    dashboard: &DefinitionId,
    folders_of: Owner,
    folder: Option<FolderId>,
) -> Statement {
    let dashboard: Value = dashboard.as_str().into();

    match (folders_of, folder) {
        (Owner::Unowned, folder) => {
            let folder: Value = folder.map(|id| id.to_string()).into();
            statement(FILE_DASHBOARD, vec![folder, dashboard])
        }
        (Owner::Person(person), Some(id)) => {
            let folder: Value = id.to_string().into();
            let mut values = vec![person.to_string().into(), folder.clone(), dashboard];
            values.extend(visible_values(person));
            values.push(folder);

            statement(&PLACE.replace("{visible}", VISIBLE), values)
        }
        (Owner::Person(person), None) => {
            statement(UNPLACE, vec![person.to_string().into(), dashboard])
        }
    }
}

/// Whether filing writes the person's own placement, which lands only on a
/// dashboard they see.
fn places(folders_of: Owner, folder: Option<FolderId>) -> bool {
    matches!((folders_of, folder), (Owner::Person(_), Some(_)))
}

fn reachable(template: &str) -> String {
    template.replace("{reachable}", REACHABLE)
}

#[async_trait]
impl Folders for MariaDefinitions {
    async fn list_folders(&self, caller: PersonId) -> Result<FolderList, FolderError> {
        let rows = SummaryRow::find_by_statement(list_statement(caller))
            .all(&self.db)
            .await?;
        let unfiled = TotalRow::find_by_statement(unfiled_statement(caller))
            .one(&self.db)
            .await?
            .map_or(0, |row| row.total);

        let folders = rows
            .iter()
            .map(|row| {
                Ok(FolderSummary {
                    folder: folder(&row.id, &row.name, row.owner_person_id.as_deref())?,
                    dashboards: u64::try_from(row.dashboards).unwrap_or(0),
                })
            })
            .collect::<Result<_, FolderError>>()?;

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

    async fn rename_folder(
        &self,
        caller: PersonId,
        id: FolderId,
        name: FolderName,
    ) -> Result<Folder, FolderError> {
        let result = self
            .db
            .execute_raw(statement(
                &reachable(RENAME_FOLDER),
                vec![
                    name.as_str().into(),
                    id.to_string().into(),
                    caller.to_string().into(),
                ],
            ))
            .await?;
        if result.rows_affected() == 0 {
            return Err(FolderError::FolderNotFound(id));
        }

        let owner = self
            .folder_owner(id)
            .await?
            .ok_or(FolderError::FolderNotFound(id))?;

        Ok(Folder { id, name, owner })
    }

    async fn delete_folder(&self, caller: PersonId, id: FolderId) -> Result<bool, FolderError> {
        let result = self
            .db
            .execute_raw(statement(
                &reachable(DELETE_FOLDER),
                vec![id.to_string().into(), caller.to_string().into()],
            ))
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn folder_owner(&self, id: FolderId) -> Result<Option<Owner>, FolderError> {
        let row = OwnerRow::find_by_statement(statement(FOLDER_OWNER, vec![id.to_string().into()]))
            .one(&self.db)
            .await?;

        match row {
            Some(row) => Ok(Some(owner_of(row.owner_person_id.as_deref())?)),
            None => Ok(None),
        }
    }

    async fn folder_of(
        &self,
        dashboard: &DefinitionId,
        caller: PersonId,
    ) -> Result<Option<Folder>, FolderError> {
        let row = FolderRow::find_by_statement(statement(
            FOLDER_OF,
            vec![
                dashboard.as_str().into(),
                dashboard.as_str().into(),
                caller.to_string().into(),
            ],
        ))
        .one(&self.db)
        .await?;

        row.map(|row| folder(&row.id, &row.name, row.owner_person_id.as_deref()))
            .transpose()
    }

    async fn file(
        &self,
        dashboard: &DefinitionId,
        folders_of: Owner,
        folder: Option<FolderId>,
    ) -> Result<(), FolderError> {
        let transaction = self.db.begin().await?;
        let held = TotalRow::find_by_statement(statement(
            COUNT_DASHBOARD,
            vec![dashboard.as_str().into()],
        ))
        .one(&transaction)
        .await?
        .map_or(0, |row| row.total);
        if held == 0 {
            return Err(FolderError::DashboardNotFound(
                dashboard.as_str().to_owned(),
            ));
        }

        let filed = transaction
            .execute_raw(file_statement(dashboard, folders_of, folder))
            .await
            .map_err(|error| match (error.sql_err(), folder) {
                (Some(sea_orm::SqlErr::ForeignKeyConstraintViolation(_)), Some(id)) => {
                    FolderError::FolderNotFound(id)
                }
                _ => error.into(),
            })?;
        if places(folders_of, folder) && filed.rows_affected() == 0 {
            return Err(FolderError::DashboardNotFound(
                dashboard.as_str().to_owned(),
            ));
        }
        transaction.commit().await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests;
