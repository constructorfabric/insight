use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait as _, DatabaseTransaction, DbErr, FromQueryResult, Statement,
    TransactionTrait as _, Value,
};
use uuid::Uuid;

use super::dashboards::owner_value;
use super::folders::{COUNT_DASHBOARD, statement};
use super::visibility::dashboard_visible;
use super::{MariaDefinitions, TotalRow};
use crate::domain::access::{Caller, Owner};
use crate::domain::definition::{DefinitionId, DefinitionStoreError};
use crate::domain::tags::{MAX_TAGS, TagError, TagName, TagSet, TagSummary, Tags};

const LOCK_TAGS: &str = "SELECT id FROM tags_lock WHERE id = 1 FOR UPDATE";

const FIND_TAG: &str = "SELECT id FROM tags WHERE owner_scope = ? AND name = ?";

const INSERT_TAG: &str = "INSERT INTO tags (id, name, owner_person_id) VALUES (?, ?, ?)";

const UNLINK_DASHBOARD: &str = "DELETE FROM dashboard_tags WHERE dashboard = ?";

const LINK_TAG: &str = "INSERT INTO dashboard_tags (dashboard, tag_id) VALUES (?, ?)";

const PRUNE_TAGS: &str = "DELETE FROM tags
WHERE NOT EXISTS (SELECT 1 FROM dashboard_tags WHERE dashboard_tags.tag_id = tags.id)";

const COUNT_TAGS: &str = "SELECT COUNT(*) AS total FROM tags WHERE owner_scope = ?";

/// One entry per name whatever scope holds it: the name column compares
/// without case, so grouping by it merges spellings, and one is kept.
const LIST_TAGS: &str = "SELECT MIN(tags.name) AS name, COUNT(DISTINCT dashboards.id) AS dashboards
FROM tags
JOIN dashboard_tags ON dashboard_tags.tag_id = tags.id
JOIN dashboards ON dashboards.id = dashboard_tags.dashboard
WHERE {visible}
GROUP BY tags.name
ORDER BY MIN(tags.name)";

const TAGS_OF: &str = "SELECT tags.name
FROM dashboard_tags JOIN tags ON tags.id = dashboard_tags.tag_id
WHERE dashboard_tags.dashboard = ?
ORDER BY tags.name";

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

/// What `owner_scope` holds for a scope: the owner, or empty for ownerless.
pub(super) fn scope_value(scope: Owner) -> Value {
    scope
        .person()
        .map_or_else(String::new, |person| person.to_string())
        .into()
}

pub(super) fn list_statement(caller: Caller) -> Statement {
    let (visible, values) = dashboard_visible(caller);

    statement(&LIST_TAGS.replace("{visible}", &visible), values)
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

async fn resolved(
    transaction: &DatabaseTransaction,
    scope: Owner,
    name: &TagName,
) -> Result<String, DbErr> {
    let held = IdRow::find_by_statement(statement(
        FIND_TAG,
        vec![scope_value(scope), name.as_str().into()],
    ))
    .one(transaction)
    .await?;
    if let Some(row) = held {
        return Ok(row.id);
    }

    let id = Uuid::now_v7().to_string();
    transaction
        .execute_raw(statement(
            INSERT_TAG,
            vec![id.clone().into(), name.as_str().into(), owner_value(scope)],
        ))
        .await?;

    Ok(id)
}

/// The ids of `names` as tags of `scope`, made where missing, each once.
async fn resolved_all(
    transaction: &DatabaseTransaction,
    scope: Owner,
    names: &[TagName],
) -> Result<Vec<String>, DbErr> {
    let mut ids: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        let id = resolved(transaction, scope, name).await?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }

    Ok(ids)
}

async fn scope_is_full(transaction: &DatabaseTransaction, scope: Owner) -> Result<bool, DbErr> {
    let total = counted(transaction, COUNT_TAGS, vec![scope_value(scope)]).await?;

    Ok(usize::try_from(total).unwrap_or(usize::MAX) > MAX_TAGS)
}

async fn names_on(
    transaction: &DatabaseTransaction,
    dashboard: &DefinitionId,
) -> Result<Vec<TagName>, DefinitionStoreError> {
    let rows = TagRow::find_by_statement(statement(TAGS_OF, vec![dashboard.as_str().into()]))
        .all(transaction)
        .await?;

    rows.iter()
        .map(|row| {
            TagName::parse(&row.name).map_err(|_| DefinitionStoreError::UnreadableRow("tag"))
        })
        .collect()
}

/// Tags `to` with the names `from` carries, as tags of `scope`.
pub(super) async fn carry_tags(
    transaction: &DatabaseTransaction,
    from: &DefinitionId,
    to: &DefinitionId,
    scope: Owner,
) -> Result<(), DefinitionStoreError> {
    let names = names_on(transaction, from).await?;
    let ids = resolved_all(transaction, scope, &names).await?;
    for id in ids {
        transaction
            .execute_raw(statement(LINK_TAG, vec![to.as_str().into(), id.into()]))
            .await?;
    }

    if scope_is_full(transaction, scope).await? {
        return Err(DefinitionStoreError::TagsFull);
    }

    Ok(())
}

/// Moves the dashboard's tags into `scope` under the same names, taking the
/// spelling `scope` already holds of a name, and prunes what the scope they
/// leave no longer uses.
pub(super) async fn rescope_tags(
    transaction: &DatabaseTransaction,
    dashboard: &DefinitionId,
    scope: Owner,
) -> Result<(), DefinitionStoreError> {
    let names = names_on(transaction, dashboard).await?;
    if names.is_empty() {
        return Ok(());
    }

    let ids = resolved_all(transaction, scope, &names).await?;
    relink(transaction, dashboard, ids).await?;
    prune_tags(transaction).await?;

    if scope_is_full(transaction, scope).await? {
        return Err(DefinitionStoreError::TagsFull);
    }

    Ok(())
}

async fn relink(
    transaction: &DatabaseTransaction,
    dashboard: &DefinitionId,
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
    async fn list_tags(&self, caller: Caller) -> Result<Vec<TagSummary>, TagError> {
        let rows = SummaryRow::find_by_statement(list_statement(caller))
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

    async fn tags_of(&self, dashboard: &DefinitionId) -> Result<Vec<TagName>, TagError> {
        let rows = TagRow::find_by_statement(statement(TAGS_OF, vec![dashboard.as_str().into()]))
            .all(&self.db)
            .await?;

        rows.iter().map(|row| TagName::parse(&row.name)).collect()
    }

    async fn set_tags(
        &self,
        dashboard: &DefinitionId,
        scope: Owner,
        tags: &TagSet,
    ) -> Result<(), TagError> {
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

        let ids = resolved_all(&transaction, scope, tags.names()).await?;
        relink(&transaction, dashboard, ids).await?;
        prune_tags(&transaction).await?;

        if scope_is_full(&transaction, scope).await? {
            return Err(TagError::TooMany);
        }
        transaction.commit().await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests;
