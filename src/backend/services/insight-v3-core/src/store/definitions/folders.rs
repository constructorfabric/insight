use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait as _, DbBackend, FromQueryResult, Statement, TransactionTrait as _, Value,
};

use super::{MariaDefinitions, NameRow, TotalRow};
use crate::domain::definition::{DefinitionName, NamePage, Page};
use crate::domain::folders::{
    Folder, FolderError, FolderFilter, FolderId, FolderList, FolderName, FolderSummary, Folders,
};
use crate::store::like_escaped;

const LIST_FOLDERS: &str = "SELECT folders.id, folders.name, COUNT(dashboards.name) AS dashboards
FROM folders LEFT JOIN dashboards ON dashboards.folder_id = folders.id
GROUP BY folders.id, folders.name
ORDER BY folders.name";

const COUNT_UNFILED: &str = "SELECT COUNT(*) AS total FROM dashboards WHERE folder_id IS NULL";

const INSERT_FOLDER: &str = "INSERT INTO folders (id, name, created_at, updated_at)
VALUES (?, ?, UTC_TIMESTAMP(6), UTC_TIMESTAMP(6))";

const RENAME_FOLDER: &str =
    "UPDATE folders SET name = ?, updated_at = UTC_TIMESTAMP(6) WHERE id = ?";

const DELETE_FOLDER: &str = "DELETE FROM folders WHERE id = ?";

const FOLDER_OF: &str = "SELECT folders.id, folders.name
FROM dashboards JOIN folders ON folders.id = dashboards.folder_id
WHERE dashboards.name = ?";

const COUNT_DASHBOARD: &str = "SELECT COUNT(*) AS total FROM dashboards WHERE name = ?";

const FILE_DASHBOARD: &str = "UPDATE dashboards SET folder_id = ? WHERE name = ?";

const PAGE_FILED: &str = "SELECT name FROM dashboards
WHERE (name LIKE ? OR body LIKE ?) AND {filter}
ORDER BY name
LIMIT ? OFFSET ?";

const COUNT_FILED: &str = "SELECT COUNT(*) AS total FROM dashboards
WHERE (name LIKE ? OR body LIKE ?) AND {filter}";

#[derive(Debug, FromQueryResult)]
struct FolderRow {
    id: String,
    name: String,
}

#[derive(Debug, FromQueryResult)]
struct SummaryRow {
    id: String,
    name: String,
    dashboards: i64,
}

fn folder(id: &str, name: &str) -> Result<Folder, FolderError> {
    Ok(Folder {
        id: FolderId::parse(id)?,
        name: FolderName::parse(name)?,
    })
}

fn statement(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::MySql, sql, values)
}

fn filtered(template: &str, filter: FolderFilter) -> String {
    let clause = match filter {
        FolderFilter::Unfiled => "folder_id IS NULL",
        FolderFilter::In(_) => "folder_id = ?",
    };
    template.replace("{filter}", clause)
}

fn matched(needle: &str, filter: FolderFilter) -> Vec<Value> {
    let pattern = format!("%{}%", like_escaped(needle));
    let mut values: Vec<Value> = vec![pattern.clone().into(), pattern.into()];
    if let FolderFilter::In(id) = filter {
        values.push(id.to_string().into());
    }
    values
}

pub(super) fn page_statement(needle: &str, page: Page, filter: FolderFilter) -> Statement {
    let mut values = matched(needle, filter);
    values.push(page.limit().into());
    values.push(page.offset().into());
    statement(&filtered(PAGE_FILED, filter), values)
}

pub(super) fn count_statement(needle: &str, filter: FolderFilter) -> Statement {
    statement(&filtered(COUNT_FILED, filter), matched(needle, filter))
}

pub(super) fn file_statement(dashboard: &DefinitionName, folder: Option<FolderId>) -> Statement {
    let folder: Value = folder.map(|id| id.to_string()).into();
    statement(FILE_DASHBOARD, vec![folder, dashboard.as_str().into()])
}

fn taken_or(error: sea_orm::DbErr, name: &FolderName) -> FolderError {
    match error.sql_err() {
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_)) => {
            FolderError::NameTaken(name.as_str().to_owned())
        }
        _ => error.into(),
    }
}

#[async_trait]
impl Folders for MariaDefinitions {
    async fn list_folders(&self) -> Result<FolderList, FolderError> {
        let rows = SummaryRow::find_by_statement(statement(LIST_FOLDERS, Vec::new()))
            .all(&self.db)
            .await?;
        let unfiled = TotalRow::find_by_statement(statement(COUNT_UNFILED, Vec::new()))
            .one(&self.db)
            .await?
            .map_or(0, |row| row.total);

        let folders = rows
            .iter()
            .map(|row| {
                Ok(FolderSummary {
                    folder: folder(&row.id, &row.name)?,
                    dashboards: u64::try_from(row.dashboards).unwrap_or(0),
                })
            })
            .collect::<Result<_, FolderError>>()?;

        Ok(FolderList {
            folders,
            unfiled: u64::try_from(unfiled).unwrap_or(0),
        })
    }

    async fn create_folder(&self, name: FolderName) -> Result<Folder, FolderError> {
        let id = FolderId::new();
        self.db
            .execute_raw(statement(
                INSERT_FOLDER,
                vec![id.to_string().into(), name.as_str().into()],
            ))
            .await
            .map_err(|error| taken_or(error, &name))?;

        Ok(Folder { id, name })
    }

    async fn rename_folder(&self, id: FolderId, name: FolderName) -> Result<Folder, FolderError> {
        let result = self
            .db
            .execute_raw(statement(
                RENAME_FOLDER,
                vec![name.as_str().into(), id.to_string().into()],
            ))
            .await
            .map_err(|error| taken_or(error, &name))?;
        if result.rows_affected() == 0 {
            return Err(FolderError::FolderNotFound);
        }

        Ok(Folder { id, name })
    }

    async fn delete_folder(&self, id: FolderId) -> Result<bool, FolderError> {
        let result = self
            .db
            .execute_raw(statement(DELETE_FOLDER, vec![id.to_string().into()]))
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn folder_of(&self, dashboard: &DefinitionName) -> Result<Option<Folder>, FolderError> {
        let row =
            FolderRow::find_by_statement(statement(FOLDER_OF, vec![dashboard.as_str().into()]))
                .one(&self.db)
                .await?;

        row.map(|row| folder(&row.id, &row.name)).transpose()
    }

    async fn file(
        &self,
        dashboard: &DefinitionName,
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
            return Err(FolderError::DashboardNotFound);
        }

        transaction
            .execute_raw(file_statement(dashboard, folder))
            .await
            .map_err(|error| match error.sql_err() {
                Some(sea_orm::SqlErr::ForeignKeyConstraintViolation(_)) => {
                    FolderError::FolderNotFound
                }
                _ => error.into(),
            })?;
        transaction.commit().await?;

        Ok(())
    }

    async fn page_filed(
        &self,
        needle: &str,
        page: Page,
        filter: FolderFilter,
    ) -> Result<NamePage, FolderError> {
        let rows = NameRow::find_by_statement(page_statement(needle, page, filter))
            .all(&self.db)
            .await?;
        let total = TotalRow::find_by_statement(count_statement(needle, filter))
            .one(&self.db)
            .await?
            .map_or(0, |row| row.total);

        Ok(NamePage {
            names: rows.into_iter().map(|row| row.name).collect(),
            total: u64::try_from(total).unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests;
