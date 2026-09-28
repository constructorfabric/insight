use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait as _, DatabaseTransaction, DbErr, FromQueryResult, Statement,
    TransactionTrait as _, Value,
};
use uuid::Uuid;

use super::folders::{COUNT_DASHBOARD, folder_clause, matched, statement};
use super::{MariaDefinitions, NameRow, TotalRow};
use crate::domain::definition::{DefinitionName, DefinitionStoreError, NamePage, Page};
use crate::domain::folders::FolderFilter;
use crate::domain::tags::{MAX_TAGS, TagError, TagFilter, TagName, TagSet, TagSummary, Tags};

const LOCK_TAGS: &str = "SELECT id FROM tags_lock WHERE id = 1 FOR UPDATE";

const FIND_TAG: &str = "SELECT id FROM tags WHERE name = ?";

const INSERT_TAG: &str = "INSERT INTO tags (id, name) VALUES (?, ?)";

const UNLINK_DASHBOARD: &str = "DELETE FROM dashboard_tags WHERE dashboard = ?";

const LINK_TAG: &str = "INSERT INTO dashboard_tags (dashboard, tag_id) VALUES (?, ?)";

const PRUNE_TAGS: &str = "DELETE FROM tags
WHERE NOT EXISTS (SELECT 1 FROM dashboard_tags WHERE dashboard_tags.tag_id = tags.id)";

const COUNT_TAGS: &str = "SELECT COUNT(*) AS total FROM tags";

const LIST_TAGS: &str = "SELECT tags.name, COUNT(*) AS dashboards
FROM tags JOIN dashboard_tags ON dashboard_tags.tag_id = tags.id
GROUP BY tags.id, tags.name
ORDER BY tags.name";

const TAGS_OF: &str = "SELECT tags.name
FROM dashboard_tags JOIN tags ON tags.id = dashboard_tags.tag_id
WHERE dashboard_tags.dashboard = ?
ORDER BY tags.name";

const PAGE_TAGGED: &str = "SELECT name FROM dashboards
WHERE (name LIKE ? OR body LIKE ?){folder} AND name IN ({tagged})
ORDER BY name
LIMIT ? OFFSET ?";

const COUNT_TAGGED: &str = "SELECT COUNT(*) AS total FROM dashboards
WHERE (name LIKE ? OR body LIKE ?){folder} AND name IN ({tagged})";

const TAGGED: &str = "SELECT dashboard_tags.dashboard
FROM dashboard_tags JOIN tags ON tags.id = dashboard_tags.tag_id
WHERE tags.name IN ({names})";

#[derive(Debug, FromQueryResult)]
struct IdRow {
    id: String,
}

#[derive(Debug, FromQueryResult)]
struct TagRow {
    name: String,
}

#[derive(Debug, FromQueryResult)]
struct SummaryRow {
    name: String,
    dashboards: i64,
}

fn narrowed(template: &str, folder: Option<FolderFilter>, tags: &TagFilter) -> String {
    let folder = folder
        .map(|filter| format!(" AND {}", folder_clause(filter)))
        .unwrap_or_default();
    let names = vec!["?"; tags.names().len()].join(", ");

    template
        .replace("{folder}", &folder)
        .replace("{tagged}", &TAGGED.replace("{names}", &names))
}

fn narrowing(needle: &str, folder: Option<FolderFilter>, tags: &TagFilter) -> Vec<Value> {
    let mut values = matched(needle, folder);
    values.extend(tags.names().iter().map(|name| name.as_str().into()));
    values
}

pub(super) fn page_statement(
    needle: &str,
    page: Page,
    folder: Option<FolderFilter>,
    tags: &TagFilter,
) -> Statement {
    let mut values = narrowing(needle, folder, tags);
    values.push(page.limit().into());
    values.push(page.offset().into());
    statement(&narrowed(PAGE_TAGGED, folder, tags), values)
}

pub(super) fn count_statement(
    needle: &str,
    folder: Option<FolderFilter>,
    tags: &TagFilter,
) -> Statement {
    statement(
        &narrowed(COUNT_TAGGED, folder, tags),
        narrowing(needle, folder, tags),
    )
}

pub(super) async fn lock_tags(transaction: &DatabaseTransaction) -> Result<(), DbErr> {
    transaction
        .query_all_raw(statement(LOCK_TAGS, Vec::new()))
        .await?;
    Ok(())
}

async fn prune_tags(transaction: &DatabaseTransaction) -> Result<(), DbErr> {
    transaction
        .execute_raw(statement(PRUNE_TAGS, Vec::new()))
        .await?;
    Ok(())
}

async fn counted(
    transaction: &DatabaseTransaction,
    sql: &str,
    values: Vec<Value>,
) -> Result<i64, DbErr> {
    Ok(TotalRow::find_by_statement(statement(sql, values))
        .one(transaction)
        .await?
        .map_or(0, |row| row.total))
}

async fn resolved(transaction: &DatabaseTransaction, name: &TagName) -> Result<String, DbErr> {
    let held = IdRow::find_by_statement(statement(FIND_TAG, vec![name.as_str().into()]))
        .one(transaction)
        .await?;
    if let Some(row) = held {
        return Ok(row.id);
    }

    let id = Uuid::now_v7().to_string();
    transaction
        .execute_raw(statement(
            INSERT_TAG,
            vec![id.clone().into(), name.as_str().into()],
        ))
        .await?;

    Ok(id)
}

async fn relink(
    transaction: &DatabaseTransaction,
    dashboard: &DefinitionName,
    ids: Vec<String>,
) -> Result<(), DbErr> {
    transaction
        .execute_raw(statement(UNLINK_DASHBOARD, vec![dashboard.as_str().into()]))
        .await?;

    for id in ids {
        transaction
            .execute_raw(statement(
                LINK_TAG,
                vec![dashboard.as_str().into(), id.into()],
            ))
            .await?;
    }

    Ok(())
}

impl MariaDefinitions {
    pub(super) async fn delete_dashboard(
        &self,
        removal: Statement,
    ) -> Result<bool, DefinitionStoreError> {
        let transaction = self.db.begin().await?;
        lock_tags(&transaction).await?;

        let removed = transaction.execute_raw(removal).await?.rows_affected() > 0;
        prune_tags(&transaction).await?;
        transaction.commit().await?;

        Ok(removed)
    }
}

#[async_trait]
impl Tags for MariaDefinitions {
    async fn list_tags(&self) -> Result<Vec<TagSummary>, TagError> {
        let rows = SummaryRow::find_by_statement(statement(LIST_TAGS, Vec::new()))
            .all(&self.db)
            .await?;

        rows.iter()
            .map(|row| {
                Ok(TagSummary {
                    name: TagName::parse(&row.name)?,
                    dashboards: u64::try_from(row.dashboards).unwrap_or(0),
                })
            })
            .collect()
    }

    async fn tags_of(&self, dashboard: &DefinitionName) -> Result<Vec<TagName>, TagError> {
        let rows = TagRow::find_by_statement(statement(TAGS_OF, vec![dashboard.as_str().into()]))
            .all(&self.db)
            .await?;

        rows.iter().map(|row| TagName::parse(&row.name)).collect()
    }

    async fn set_tags(&self, dashboard: &DefinitionName, tags: &TagSet) -> Result<(), TagError> {
        let transaction = self.db.begin().await?;
        lock_tags(&transaction).await?;
        let held = counted(
            &transaction,
            COUNT_DASHBOARD,
            vec![dashboard.as_str().into()],
        )
        .await?;
        if held == 0 {
            return Err(TagError::DashboardNotFound(dashboard.as_str().to_owned()));
        }

        let mut ids: Vec<String> = Vec::with_capacity(tags.names().len());
        for name in tags.names() {
            let id = resolved(&transaction, name).await?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        relink(&transaction, dashboard, ids).await?;
        prune_tags(&transaction).await?;

        let total = counted(&transaction, COUNT_TAGS, Vec::new()).await?;
        if usize::try_from(total).unwrap_or(usize::MAX) > MAX_TAGS {
            return Err(TagError::TooMany);
        }
        transaction.commit().await?;

        Ok(())
    }

    async fn page_tagged(
        &self,
        needle: &str,
        page: Page,
        folder: Option<FolderFilter>,
        tags: &TagFilter,
    ) -> Result<NamePage, TagError> {
        let rows = NameRow::find_by_statement(page_statement(needle, page, folder, tags))
            .all(&self.db)
            .await?;
        let total = TotalRow::find_by_statement(count_statement(needle, folder, tags))
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
