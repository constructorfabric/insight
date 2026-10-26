use async_trait::async_trait;
use chrono::NaiveDateTime;
use sea_orm::{
    ConnectionTrait as _, DatabaseTransaction, DbErr, FromQueryResult, Statement,
    TransactionTrait as _, Value,
};

use super::dashboards::level_of;
use super::folders::statement;
use super::{MariaDefinitions, TotalRow, tags};
use crate::domain::access::{Grantee, Level, Owner, PersonId};
use crate::domain::definition::{DefinitionId, DefinitionStoreError, Page};
use crate::domain::folders::FolderId;
use crate::domain::sharing::{Grant, Grants, Handover, Orphan, OrphanPage, Shared};

const GRANTS_OF: &str = "SELECT grantee, level, granted_by, created_at FROM {table}
WHERE {key} = ?
ORDER BY created_at, grantee";

/// Granting again sets the level; who granted it first and when stay.
const GRANT: &str = "INSERT INTO {table} ({key}, grantee, level, granted_by, created_at)
VALUES (?, ?, ?, ?, UTC_TIMESTAMP(6))
ON DUPLICATE KEY UPDATE level = ?";

const WITHDRAW: &str = "DELETE FROM {table} WHERE {key} = ? AND grantee = ?";

/// Stops each carrying entry of the dashboard whose placer is neither its
/// owner nor an editor by a grant to them or to everyone from carrying.
const DEMOTE_ENTRIES: &str = "UPDATE folder_entries SET carries = FALSE
WHERE folder_entries.dashboard = ? AND folder_entries.carries
AND folder_entries.placed_by IS NOT NULL
AND NOT EXISTS (SELECT 1 FROM dashboards placed
WHERE placed.id = folder_entries.dashboard AND placed.owner_person_id = folder_entries.placed_by)
AND NOT EXISTS (SELECT 1 FROM dashboard_grants editors
WHERE editors.dashboard = folder_entries.dashboard AND editors.level = 'editor'
AND editors.grantee IN (folder_entries.placed_by, 'everyone'))";

/// Conditional on the owner the caller read, so of two transfers racing
/// only one moves it.
const HAND_OVER: &str =
    "UPDATE {owned} SET owner_person_id = ? WHERE id = ? AND owner_person_id = ?";

const PAGE_ORPHANED: &str = "SELECT id, name, owner_person_id, updated_at FROM {owned}
WHERE {orphaned}
ORDER BY name, id
LIMIT ? OFFSET ?";

const COUNT_ORPHANED: &str = "SELECT COUNT(*) AS total FROM {owned} WHERE {orphaned}";

/// A folder orphans only once it is shared with someone.
const SHARED_FOLDER: &str =
    "EXISTS (SELECT 1 FROM folder_grants WHERE folder_grants.folder_id = folders.id)";

#[derive(Debug, FromQueryResult)]
struct GrantRow {
    grantee: String,
    level: String,
    granted_by: String,
    created_at: NaiveDateTime,
}

#[derive(Debug, FromQueryResult)]
struct OrphanRow {
    id: String,
    name: String,
    owner_person_id: String,
    updated_at: NaiveDateTime,
}

/// Which owned table an orphan listing or a hand-over reads.
#[derive(Debug, Clone, Copy)]
pub(super) enum Owned {
    Dashboards,
    Folders,
}

impl Owned {
    fn table(self) -> &'static str {
        match self {
            Self::Dashboards => "dashboards",
            Self::Folders => "folders",
        }
    }

    fn orphaned_only(self) -> Option<&'static str> {
        match self {
            Self::Dashboards => None,
            Self::Folders => Some(SHARED_FOLDER),
        }
    }
}

/// `{table}` and `{key}` are substituted from our own enum, never from
/// anything a request carries.
fn on(template: &str, shared: Shared<'_>) -> String {
    let (table, key) = match shared {
        Shared::Dashboard(_) => ("dashboard_grants", "dashboard"),
        Shared::Folder(_) => ("folder_grants", "folder_id"),
    };

    template.replace("{table}", table).replace("{key}", key)
}

fn key(shared: Shared<'_>) -> Value {
    match shared {
        Shared::Dashboard(id) => id.as_str().into(),
        Shared::Folder(id) => id.to_string().into(),
    }
}

fn person(raw: &str) -> Result<PersonId, DefinitionStoreError> {
    PersonId::parse(raw).ok_or(DefinitionStoreError::UnreadableRow("person"))
}

pub(super) fn grant_statement(
    shared: Shared<'_>,
    grantee: Grantee,
    level: Level,
    granted_by: PersonId,
) -> Statement {
    statement(
        &on(GRANT, shared),
        vec![
            key(shared),
            grantee.to_string().into(),
            level.as_str().into(),
            granted_by.to_string().into(),
            level.as_str().into(),
        ],
    )
}

pub(super) fn demote_statement(dashboard: &DefinitionId) -> Statement {
    statement(DEMOTE_ENTRIES, vec![dashboard.as_str().into()])
}

/// The clause keeping owned rows whose owner is none of `administrators`,
/// with its values.
pub(super) fn orphaned_clause(owned: Owned, administrators: &[PersonId]) -> (String, Vec<Value>) {
    let mut clauses = vec!["owner_person_id IS NOT NULL".to_owned()];
    let values: Vec<Value> = administrators
        .iter()
        .map(|person| person.to_string().into())
        .collect();
    if !administrators.is_empty() {
        let marks = vec!["?"; administrators.len()].join(", ");
        clauses.push(format!("owner_person_id NOT IN ({marks})"));
    }
    clauses.extend(owned.orphaned_only().map(str::to_owned));

    (clauses.join(" AND "), values)
}

pub(super) fn orphaned_page_statement(
    owned: Owned,
    administrators: &[PersonId],
    page: Page,
) -> Statement {
    let (clause, mut values) = orphaned_clause(owned, administrators);
    values.push(page.limit().into());
    values.push(page.offset().into());

    let sql = PAGE_ORPHANED
        .replace("{owned}", owned.table())
        .replace("{orphaned}", &clause);

    statement(&sql, values)
}

fn orphaned_count_statement(owned: Owned, administrators: &[PersonId]) -> Statement {
    let (clause, values) = orphaned_clause(owned, administrators);
    let sql = COUNT_ORPHANED
        .replace("{owned}", owned.table())
        .replace("{orphaned}", &clause);

    statement(&sql, values)
}

fn missing_or(error: DbErr, shared: Shared<'_>) -> DefinitionStoreError {
    match error.sql_err() {
        Some(sea_orm::SqlErr::ForeignKeyConstraintViolation(_)) => {
            let id = match shared {
                Shared::Dashboard(id) => id.as_str().to_owned(),
                Shared::Folder(id) => id.to_string(),
            };
            DefinitionStoreError::Missing(id)
        }
        _ => DefinitionStoreError::Database(error),
    }
}

/// Keeps every carrying entry of a dashboard true to its grants and owner.
async fn demote(transaction: &DatabaseTransaction, shared: Shared<'_>) -> Result<(), DbErr> {
    let Shared::Dashboard(dashboard) = shared else {
        return Ok(());
    };
    transaction.execute_raw(demote_statement(dashboard)).await?;

    Ok(())
}

async fn hand_over(
    transaction: &DatabaseTransaction,
    owned: Owned,
    id: &str,
    from: PersonId,
    to: PersonId,
) -> Result<bool, DbErr> {
    let moved = transaction
        .execute_raw(statement(
            &HAND_OVER.replace("{owned}", owned.table()),
            vec![to.to_string().into(), id.into(), from.to_string().into()],
        ))
        .await?;

    Ok(moved.rows_affected() > 0)
}

impl MariaDefinitions {
    async fn page_of(
        &self,
        owned: Owned,
        administrators: &[PersonId],
        page: Page,
    ) -> Result<OrphanPage, DefinitionStoreError> {
        let rows =
            OrphanRow::find_by_statement(orphaned_page_statement(owned, administrators, page))
                .all(&self.db)
                .await?;
        let total = TotalRow::find_by_statement(orphaned_count_statement(owned, administrators))
            .one(&self.db)
            .await?
            .map_or(0, |row| row.total);

        let items = rows
            .into_iter()
            .map(|row| {
                Ok(Orphan {
                    id: row.id,
                    name: row.name,
                    owner: person(&row.owner_person_id)?,
                    updated_at: row.updated_at.and_utc(),
                })
            })
            .collect::<Result<_, DefinitionStoreError>>()?;

        Ok(OrphanPage {
            items,
            total: u64::try_from(total).unwrap_or(0),
        })
    }
}

#[async_trait]
impl Grants for MariaDefinitions {
    async fn grants_of(&self, shared: Shared<'_>) -> Result<Vec<Grant>, DefinitionStoreError> {
        let rows =
            GrantRow::find_by_statement(statement(&on(GRANTS_OF, shared), vec![key(shared)]))
                .all(&self.db)
                .await?;

        rows.into_iter()
            .map(|row| {
                Ok(Grant {
                    grantee: Grantee::parse(&row.grantee)
                        .ok_or(DefinitionStoreError::UnreadableRow("grantee"))?,
                    level: level_of(&row.level)?,
                    granted_by: person(&row.granted_by)?,
                    created_at: row.created_at.and_utc(),
                })
            })
            .collect()
    }

    async fn grant(
        &self,
        shared: Shared<'_>,
        grantee: Grantee,
        level: Level,
        granted_by: PersonId,
    ) -> Result<(), DefinitionStoreError> {
        let transaction = self.db.begin().await?;
        transaction
            .execute_raw(grant_statement(shared, grantee, level, granted_by))
            .await
            .map_err(|error| missing_or(error, shared))?;
        demote(&transaction, shared).await?;
        transaction.commit().await?;

        Ok(())
    }

    async fn withdraw(
        &self,
        shared: Shared<'_>,
        grantee: Grantee,
    ) -> Result<(), DefinitionStoreError> {
        let transaction = self.db.begin().await?;
        transaction
            .execute_raw(statement(
                &on(WITHDRAW, shared),
                vec![key(shared), grantee.to_string().into()],
            ))
            .await?;
        demote(&transaction, shared).await?;
        transaction.commit().await?;

        Ok(())
    }

    async fn transfer(
        &self,
        dashboard: &DefinitionId,
        from: PersonId,
        to: PersonId,
    ) -> Result<Handover, DefinitionStoreError> {
        let shared = Shared::Dashboard(dashboard);
        let transaction = self.db.begin().await?;
        tags::lock_tags(&transaction).await?;
        if !hand_over(
            &transaction,
            Owned::Dashboards,
            dashboard.as_str(),
            from,
            to,
        )
        .await?
        {
            return Ok(Handover::OwnerChanged);
        }

        transaction
            .execute_raw(statement(
                &on(WITHDRAW, shared),
                vec![key(shared), to.to_string().into()],
            ))
            .await?;
        demote(&transaction, shared).await?;
        match tags::rescope_tags(&transaction, dashboard, Owner::Person(to)).await {
            Ok(()) => {}
            Err(DefinitionStoreError::TagsFull) => return Ok(Handover::RecipientTagsFull),
            Err(error) => return Err(error),
        }
        transaction.commit().await?;

        Ok(Handover::Handed)
    }

    async fn transfer_folder(
        &self,
        folder: FolderId,
        from: PersonId,
        to: PersonId,
    ) -> Result<Handover, DefinitionStoreError> {
        let shared = Shared::Folder(folder);
        let transaction = self.db.begin().await?;
        if !hand_over(&transaction, Owned::Folders, &folder.to_string(), from, to).await? {
            return Ok(Handover::OwnerChanged);
        }

        transaction
            .execute_raw(statement(
                &on(WITHDRAW, shared),
                vec![key(shared), to.to_string().into()],
            ))
            .await?;
        transaction.commit().await?;

        Ok(Handover::Handed)
    }

    async fn page_orphaned(
        &self,
        administrators: &[PersonId],
        page: Page,
    ) -> Result<OrphanPage, DefinitionStoreError> {
        self.page_of(Owned::Dashboards, administrators, page).await
    }

    async fn page_orphaned_folders(
        &self,
        administrators: &[PersonId],
        page: Page,
    ) -> Result<OrphanPage, DefinitionStoreError> {
        self.page_of(Owned::Folders, administrators, page).await
    }
}
