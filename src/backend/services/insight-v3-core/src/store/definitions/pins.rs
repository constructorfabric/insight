use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait as _, DatabaseTransaction, FromQueryResult, Statement, TransactionTrait as _,
};

use super::dashboards::{VISIBLE, visible_values};
use super::folders::{COUNT_DASHBOARD, statement};
use super::{ListedRow, MariaDefinitions, TotalRow};
use crate::domain::access::PersonId;
use crate::domain::definition::{DefinitionId, Listed};
use crate::domain::pins::{MAX_PINS, PinError, Pins};

const LOCK_PINS: &str = "SELECT id FROM pins_lock WHERE id = 1 FOR UPDATE";

const PINS_OF: &str = "SELECT dashboard_pins.dashboard AS id, dashboards.name
FROM dashboard_pins JOIN dashboards ON dashboards.id = dashboard_pins.dashboard
WHERE dashboard_pins.person = ? AND {visible}
ORDER BY dashboard_pins.pinned_at, dashboard_pins.dashboard";

const IS_PINNED: &str =
    "SELECT COUNT(*) AS total FROM dashboard_pins WHERE person = ? AND dashboard = ?";

/// Only the pins the person still sees count against the cap, since the
/// rest are neither listed nor reachable to unpin.
const COUNT_PINS: &str = "SELECT COUNT(*) AS total
FROM dashboard_pins JOIN dashboards ON dashboards.id = dashboard_pins.dashboard
WHERE dashboard_pins.person = ? AND {visible}";

const INSERT_PIN: &str = "INSERT INTO dashboard_pins (person, dashboard, pinned_at)
VALUES (?, ?, UTC_TIMESTAMP(6))";

const DELETE_PIN: &str = "DELETE FROM dashboard_pins WHERE person = ? AND dashboard = ?";

async fn counted(
    transaction: &DatabaseTransaction,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Result<i64, PinError> {
    Ok(TotalRow::find_by_statement(statement(sql, values))
        .one(transaction)
        .await?
        .map_or(0, |row| row.total))
}

fn not_found(dashboard: &DefinitionId) -> PinError {
    PinError::DashboardNotFound(dashboard.as_str().to_owned())
}

/// A query over one person's pins that they still see.
pub(super) fn visible_pins(template: &str, person: PersonId) -> Statement {
    let mut values: Vec<sea_orm::Value> = vec![person.to_string().into()];
    values.extend(visible_values(person));

    statement(&template.replace("{visible}", VISIBLE), values)
}

#[async_trait]
impl Pins for MariaDefinitions {
    async fn pins_of(&self, person: PersonId) -> Result<Vec<Listed>, PinError> {
        let rows = ListedRow::find_by_statement(visible_pins(PINS_OF, person))
            .all(&self.db)
            .await?;

        Ok(rows.into_iter().map(ListedRow::into_listed).collect())
    }

    async fn pin(&self, person: PersonId, dashboard: &DefinitionId) -> Result<(), PinError> {
        let transaction = self.db.begin().await?;
        transaction
            .query_all_raw(statement(LOCK_PINS, Vec::new()))
            .await?;
        let pinning = vec![person.to_string().into(), dashboard.as_str().into()];

        let held = counted(
            &transaction,
            COUNT_DASHBOARD,
            vec![dashboard.as_str().into()],
        )
        .await?;
        if held == 0 {
            return Err(not_found(dashboard));
        }
        if counted(&transaction, IS_PINNED, pinning.clone()).await? > 0 {
            return Ok(());
        }

        let pins = TotalRow::find_by_statement(visible_pins(COUNT_PINS, person))
            .one(&transaction)
            .await?
            .map_or(0, |row| row.total);
        if usize::try_from(pins).unwrap_or(usize::MAX) >= MAX_PINS {
            return Err(PinError::TooMany);
        }
        transaction
            .execute_raw(statement(INSERT_PIN, pinning))
            .await
            .map_err(|error| match error.sql_err() {
                Some(sea_orm::SqlErr::ForeignKeyConstraintViolation(_)) => not_found(dashboard),
                _ => error.into(),
            })?;
        transaction.commit().await?;

        Ok(())
    }

    async fn unpin(&self, person: PersonId, dashboard: &DefinitionId) -> Result<bool, PinError> {
        let removed = self
            .db
            .execute_raw(statement(
                DELETE_PIN,
                vec![person.to_string().into(), dashboard.as_str().into()],
            ))
            .await?;

        Ok(removed.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests;
