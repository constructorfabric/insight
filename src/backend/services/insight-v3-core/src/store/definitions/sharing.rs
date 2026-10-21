use async_trait::async_trait;
use chrono::NaiveDateTime;
use sea_orm::{ConnectionTrait as _, FromQueryResult, Statement, TransactionTrait as _, Value};

use super::folders::statement;
use super::{MariaDefinitions, TotalRow};
use crate::domain::access::PersonId;
use crate::domain::definition::{DefinitionId, DefinitionStoreError, Page};
use crate::domain::sharing::{Orphan, OrphanPage, Share, Sharing};

const SHARES_OF: &str = "SELECT person, granted_by, created_at FROM dashboard_shares
WHERE dashboard = ?
ORDER BY created_at, person";

/// A share already held keeps who granted it and when.
const SHARE: &str = "INSERT INTO dashboard_shares (dashboard, person, granted_by, created_at)
VALUES (?, ?, ?, UTC_TIMESTAMP(6))
ON DUPLICATE KEY UPDATE person = person";

const UNSHARE: &str = "DELETE FROM dashboard_shares WHERE dashboard = ? AND person = ?";

const UNPIN: &str = "DELETE FROM dashboard_pins WHERE dashboard = ? AND person = ?";

const UNPLACE: &str = "DELETE FROM dashboard_placements WHERE dashboard = ? AND person = ?";

/// Conditional on the owner the caller read, so of two transfers racing
/// only one moves the dashboard.
const HAND_OVER: &str =
    "UPDATE dashboards SET owner_person_id = ? WHERE id = ? AND owner_person_id = ?";

const PAGE_ORPHANED: &str = "SELECT id, name, owner_person_id, updated_at FROM dashboards
WHERE {orphaned}
ORDER BY name, id
LIMIT ? OFFSET ?";

const COUNT_ORPHANED: &str = "SELECT COUNT(*) AS total FROM dashboards WHERE {orphaned}";

#[derive(Debug, FromQueryResult)]
struct ShareRow {
    person: String,
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

fn person(raw: &str) -> Result<PersonId, DefinitionStoreError> {
    PersonId::parse(raw).ok_or(DefinitionStoreError::UnreadableRow("person"))
}

fn pair(dashboard: &DefinitionId, person: PersonId) -> Vec<Value> {
    vec![dashboard.as_str().into(), person.to_string().into()]
}

/// The clause keeping owned dashboards whose owner is none of
/// `administrators`, with its values.
pub(super) fn orphaned_clause(administrators: &[PersonId]) -> (String, Vec<Value>) {
    if administrators.is_empty() {
        return ("owner_person_id IS NOT NULL".to_owned(), Vec::new());
    }

    let marks = vec!["?"; administrators.len()].join(", ");
    let values = administrators
        .iter()
        .map(|person| person.to_string().into())
        .collect();

    (
        format!("owner_person_id IS NOT NULL AND owner_person_id NOT IN ({marks})"),
        values,
    )
}

pub(super) fn orphaned_page_statement(administrators: &[PersonId], page: Page) -> Statement {
    let (clause, mut values) = orphaned_clause(administrators);
    values.push(page.limit().into());
    values.push(page.offset().into());

    statement(&PAGE_ORPHANED.replace("{orphaned}", &clause), values)
}

fn orphaned_count_statement(administrators: &[PersonId]) -> Statement {
    let (clause, values) = orphaned_clause(administrators);

    statement(&COUNT_ORPHANED.replace("{orphaned}", &clause), values)
}

fn missing_or(error: sea_orm::DbErr, dashboard: &DefinitionId) -> DefinitionStoreError {
    match error.sql_err() {
        Some(sea_orm::SqlErr::ForeignKeyConstraintViolation(_)) => {
            DefinitionStoreError::Missing(dashboard.as_str().to_owned())
        }
        _ => DefinitionStoreError::Database(error),
    }
}

#[async_trait]
impl Sharing for MariaDefinitions {
    async fn shares_of(
        &self,
        dashboard: &DefinitionId,
    ) -> Result<Vec<Share>, DefinitionStoreError> {
        let rows =
            ShareRow::find_by_statement(statement(SHARES_OF, vec![dashboard.as_str().into()]))
                .all(&self.db)
                .await?;

        rows.into_iter()
            .map(|row| {
                Ok(Share {
                    person: person(&row.person)?,
                    granted_by: person(&row.granted_by)?,
                    created_at: row.created_at.and_utc(),
                })
            })
            .collect()
    }

    async fn share(
        &self,
        dashboard: &DefinitionId,
        person: PersonId,
        granted_by: PersonId,
    ) -> Result<(), DefinitionStoreError> {
        let mut values = pair(dashboard, person);
        values.push(granted_by.to_string().into());

        self.db
            .execute_raw(statement(SHARE, values))
            .await
            .map_err(|error| missing_or(error, dashboard))?;

        Ok(())
    }

    async fn withdraw(
        &self,
        dashboard: &DefinitionId,
        person: PersonId,
    ) -> Result<(), DefinitionStoreError> {
        let transaction = self.db.begin().await?;
        for sql in [UNSHARE, UNPIN, UNPLACE] {
            transaction
                .execute_raw(statement(sql, pair(dashboard, person)))
                .await?;
        }
        transaction.commit().await?;

        Ok(())
    }

    async fn transfer(
        &self,
        dashboard: &DefinitionId,
        from: PersonId,
        to: PersonId,
    ) -> Result<bool, DefinitionStoreError> {
        let transaction = self.db.begin().await?;
        let moved = transaction
            .execute_raw(statement(
                HAND_OVER,
                vec![
                    to.to_string().into(),
                    dashboard.as_str().into(),
                    from.to_string().into(),
                ],
            ))
            .await?;
        if moved.rows_affected() == 0 {
            return Ok(false);
        }

        transaction
            .execute_raw(statement(UNSHARE, pair(dashboard, to)))
            .await?;
        transaction.commit().await?;

        Ok(true)
    }

    async fn page_orphaned(
        &self,
        administrators: &[PersonId],
        page: Page,
    ) -> Result<OrphanPage, DefinitionStoreError> {
        let rows = OrphanRow::find_by_statement(orphaned_page_statement(administrators, page))
            .all(&self.db)
            .await?;
        let total = TotalRow::find_by_statement(orphaned_count_statement(administrators))
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
