//! The dataset rows as `MariaDB` holds them.

#[cfg(test)]
pub(crate) mod memory;

use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait as _, DatabaseConnection, DbBackend, FromQueryResult, Statement,
    TransactionTrait as _,
};
use serde_json::Value;

use crate::domain::datasets::{
    Attempt, Dataset, DatasetStoreError, Datasets, Finish, Held, OperationToken, Owning, Refused,
    Taking, finishing, lease_until, taking,
};
use crate::domain::definition::DefinitionName;
use crate::domain::kinds::dataset::lifecycle::{DatasetState, Operation};

const SELECT_ROW: &str = "SELECT name, body, state, physical_table, operation, operation_token, lease_until FROM datasets WHERE name = ?";
/// The same read, holding the row: a second writer of this dataset waits
/// rather than acting on a state that is about to change.
const SELECT_ROW_HELD: &str = "SELECT name, body, state, physical_table, operation, operation_token, lease_until FROM datasets WHERE name = ? FOR UPDATE";
const SELECT_NAMES: &str = "SELECT name FROM datasets ORDER BY name";
/// One clock for every lease, so two servers cannot disagree about whether
/// one has lapsed.
const NOW: &str = "SELECT UTC_TIMESTAMP(6) AS now";
const CLAIM_NAME: &str = "INSERT INTO datasets (name, body, state, operation, operation_token, lease_until, updated_at) VALUES (?, ?, ?, ?, ?, ?, UTC_TIMESTAMP(6))";
const TAKE_OPERATION: &str = "UPDATE datasets SET body = ?, state = ?, operation = ?, operation_token = ?, lease_until = ?, updated_at = UTC_TIMESTAMP(6) WHERE name = ?";
/// A removal keeps the declaration it found: it publishes nothing.
const TAKE_REMOVAL: &str = "UPDATE datasets SET state = ?, operation = ?, operation_token = ?, lease_until = ?, updated_at = UTC_TIMESTAMP(6) WHERE name = ?";
const RECORD_TABLE: &str =
    "UPDATE datasets SET physical_table = ?, updated_at = UTC_TIMESTAMP(6) WHERE name = ?";
const MARK_READY: &str = "UPDATE datasets SET state = ?, operation = NULL, operation_token = NULL, lease_until = NULL, updated_at = UTC_TIMESTAMP(6) WHERE name = ?";
const DELETE_ROW: &str = "DELETE FROM datasets WHERE name = ?";

pub(crate) struct MariaDatasets {
    db: DatabaseConnection,
}

impl MariaDatasets {
    pub(crate) fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Datasets for MariaDatasets {
    async fn get(&self, name: &DefinitionName) -> Result<Option<Dataset>, DatasetStoreError> {
        let found = DatasetRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::MySql,
            SELECT_ROW,
            [name.as_str().into()],
        ))
        .one(&self.db)
        .await?;

        found.map(DatasetRow::into_dataset).transpose()
    }

    async fn list(&self) -> Result<Vec<String>, DatasetStoreError> {
        let rows = NameRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::MySql,
            SELECT_NAMES,
            [],
        ))
        .all(&self.db)
        .await?;

        Ok(rows.into_iter().map(|row| row.name).collect())
    }

    async fn take_create(
        &self,
        name: &DefinitionName,
        declaration: &Value,
    ) -> Result<Attempt, DatasetStoreError> {
        self.take(name, Operation::Create, Some(declaration)).await
    }

    async fn take_remove(&self, name: &DefinitionName) -> Result<Attempt, DatasetStoreError> {
        self.take(name, Operation::Remove, None).await
    }

    async fn finish(
        &self,
        name: &DefinitionName,
        token: &OperationToken,
        finish: Finish,
    ) -> Result<Owning, DatasetStoreError> {
        let transaction = self.db.begin().await?;

        let held = held_row(&transaction, name).await?;
        if finishing(held.as_ref(), token) == Owning::Lost {
            return Ok(Owning::Lost);
        }

        let statement = match finish {
            Finish::Provisioned(table) => Statement::from_sql_and_values(
                DbBackend::MySql,
                RECORD_TABLE,
                [table.into(), name.as_str().into()],
            ),
            Finish::Ready => Statement::from_sql_and_values(
                DbBackend::MySql,
                MARK_READY,
                [DatasetState::Ready.as_str().into(), name.as_str().into()],
            ),
            Finish::Removed => {
                Statement::from_sql_and_values(DbBackend::MySql, DELETE_ROW, [name.as_str().into()])
            }
        };

        transaction.execute_raw(statement).await?;
        transaction.commit().await?;

        Ok(Owning::Held)
    }
}

impl MariaDatasets {
    async fn take(
        &self,
        name: &DefinitionName,
        operation: Operation,
        declaration: Option<&Value>,
    ) -> Result<Attempt, DatasetStoreError> {
        let transaction = self.db.begin().await?;

        let now =
            NowRow::find_by_statement(Statement::from_sql_and_values(DbBackend::MySql, NOW, []))
                .one(&transaction)
                .await?
                .ok_or_else(|| sea_orm::DbErr::Custom("the store has no clock".to_owned()))?
                .now
                .and_utc();

        let held = held_row(&transaction, name).await?;

        let token = OperationToken::mint();
        let until = lease_until(now);

        let state = match taking(held.as_ref(), operation, now) {
            Taking::Refuse(refusal) => return Err(refusal.into()),
            Taking::Gone => return Err(Refused::Gone.into()),
            Taking::Claim => {
                let body =
                    declaration.map_or_else(|| Ok(String::from("{}")), serde_json::to_string)?;
                transaction
                    .execute_raw(Statement::from_sql_and_values(
                        DbBackend::MySql,
                        CLAIM_NAME,
                        [
                            name.as_str().into(),
                            body.into(),
                            DatasetState::Claimed.as_str().into(),
                            operation.as_str().into(),
                            token.as_str().into(),
                            until.naive_utc().into(),
                        ],
                    ))
                    .await
                    .map_err(taken_or)?;

                DatasetState::Claimed
            }
            Taking::Take(state) => {
                let statement = match declaration {
                    Some(declaration) => Statement::from_sql_and_values(
                        DbBackend::MySql,
                        TAKE_OPERATION,
                        [
                            serde_json::to_string(declaration)?.into(),
                            state.as_str().into(),
                            operation.as_str().into(),
                            token.as_str().into(),
                            until.naive_utc().into(),
                            name.as_str().into(),
                        ],
                    ),
                    None => Statement::from_sql_and_values(
                        DbBackend::MySql,
                        TAKE_REMOVAL,
                        [
                            state.as_str().into(),
                            operation.as_str().into(),
                            token.as_str().into(),
                            until.naive_utc().into(),
                            name.as_str().into(),
                        ],
                    ),
                };
                transaction.execute_raw(statement).await?;

                state
            }
        };

        transaction.commit().await?;

        Ok(Attempt { token, state })
    }
}

/// The row, held for the rest of the transaction.
async fn held_row(
    transaction: &sea_orm::DatabaseTransaction,
    name: &DefinitionName,
) -> Result<Option<Dataset>, DatasetStoreError> {
    DatasetRow::find_by_statement(Statement::from_sql_and_values(
        DbBackend::MySql,
        SELECT_ROW_HELD,
        [name.as_str().into()],
    ))
    .one(transaction)
    .await?
    .map(DatasetRow::into_dataset)
    .transpose()
}

/// A name another attempt inserted first is the same refusal as a held
/// operation: of two attempts at one new dataset, exactly one inserts.
fn taken_or(error: sea_orm::DbErr) -> DatasetStoreError {
    match error.sql_err() {
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_)) => {
            Refused::Busy(Operation::Create).into()
        }
        _ => DatasetStoreError::Database(error),
    }
}

impl std::fmt::Debug for MariaDatasets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MariaDatasets")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, FromQueryResult)]
struct DatasetRow {
    name: String,
    body: String,
    state: String,
    physical_table: Option<String>,
    operation: Option<String>,
    operation_token: Option<String>,
    lease_until: Option<chrono::NaiveDateTime>,
}

impl DatasetRow {
    fn into_dataset(self) -> Result<Dataset, DatasetStoreError> {
        let state = DatasetState::parse(&self.state)
            .ok_or_else(|| DatasetStoreError::UnreadableRow(self.state.clone()))?;
        let name = DefinitionName::parse(&self.name)
            .map_err(|_| DatasetStoreError::UnreadableRow(self.name.clone()))?;

        let held = match (self.operation, self.operation_token, self.lease_until) {
            (Some(operation), Some(token), Some(until)) => Some(Held {
                operation: Operation::parse(&operation)
                    .ok_or(DatasetStoreError::UnreadableRow(operation))?,
                token: OperationToken::from_row(token),
                until: until.and_utc(),
            }),
            _ => None,
        };

        Ok(Dataset {
            name,
            declaration: serde_json::from_str(&self.body)?,
            state,
            physical_table: self.physical_table,
            held,
        })
    }
}

#[derive(Debug, FromQueryResult)]
struct NameRow {
    name: String,
}

#[derive(Debug, FromQueryResult)]
struct NowRow {
    now: chrono::NaiveDateTime,
}

#[cfg(test)]
mod tests;
