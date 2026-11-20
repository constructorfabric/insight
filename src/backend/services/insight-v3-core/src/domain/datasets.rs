use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::Value;
use thiserror::Error;

use super::definition::{DefinitionId, Page};
use super::kinds::dataset::declaration::Declaration;
use super::kinds::dataset::state::{DatasetState, Operation};

pub(crate) const LEASE_SECS: i64 = 60;

const TABLE_PREFIX: &str = "ds_";
const TABLE_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperationToken(String);

impl OperationToken {
    pub(crate) fn mint() -> Self {
        Self(uuid::Uuid::now_v7().simple().to_string())
    }

    pub(crate) fn from_row(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn table(&self, dataset: &DefinitionId) -> String {
        let readable: String = dataset.as_str().chars().take(TABLE_NAME_CHARS).collect();

        format!("{TABLE_PREFIX}{readable}_{}", self.0)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Held {
    pub(crate) operation: Operation,
    pub(crate) token: OperationToken,
    pub(crate) until: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub(crate) struct Dataset {
    pub(crate) name: DefinitionId,
    pub(crate) declaration: Value,
    pub(crate) state: DatasetState,
    pub(crate) physical_table: Option<String>,
    pub(crate) held: Option<Held>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug)]
pub(crate) struct DatasetPage {
    pub(crate) datasets: Vec<Dataset>,
    pub(crate) total: u64,
}

impl Dataset {
    pub(crate) fn holder(&self, now: DateTime<Utc>) -> Option<Operation> {
        self.held
            .as_ref()
            .filter(|held| held.until > now)
            .map(|held| held.operation)
    }
}

#[derive(Debug)]
pub(crate) struct Ready {
    pub(crate) declaration: Declaration,
    pub(crate) table: String,
}

pub(crate) async fn ready(
    datasets: &dyn Datasets,
    named: &str,
) -> Result<Option<Ready>, DatasetStoreError> {
    let Ok(name) = DefinitionId::parse(named) else {
        return Ok(None);
    };
    let Some(held) = datasets.get(&name).await? else {
        return Ok(None);
    };
    if held.state != DatasetState::Ready {
        return Ok(None);
    }
    let Some(table) = held.physical_table else {
        return Ok(None);
    };

    Ok(Some(Ready {
        declaration: serde_json::from_value(held.declaration)?,
        table,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum Refused {
    #[error("a {0} of this dataset is already under way")]
    Busy(Operation),
    #[error("there is no such dataset")]
    Gone,
    #[error("this name belongs to a removal until it finishes")]
    Removing,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Taking {
    Claim,
    Gone,
    Stands,
    Take(DatasetState),
    Refuse(Refused),
}

pub(crate) fn taking(held: Option<&Dataset>, operation: Operation, now: DateTime<Utc>) -> Taking {
    let Some(held) = held else {
        return match operation {
            Operation::Create => Taking::Claim,
            Operation::Remove => Taking::Gone,
        };
    };

    if let Some(holder) = held.holder(now) {
        return Taking::Refuse(Refused::Busy(holder));
    }

    if held.state == DatasetState::Removing && operation == Operation::Create {
        return Taking::Refuse(Refused::Removing);
    }

    if held.state == DatasetState::Ready && operation == Operation::Create {
        return Taking::Stands;
    }

    Taking::Take(match operation {
        Operation::Create => DatasetState::Claimed,
        Operation::Remove => DatasetState::Removing,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Lease(TimeDelta);

impl Lease {
    pub(crate) fn of_seconds(seconds: i64) -> Self {
        Self(TimeDelta::seconds(seconds))
    }

    pub(crate) fn until(self, now: DateTime<Utc>) -> DateTime<Utc> {
        now + self.0
    }
}

impl Default for Lease {
    fn default() -> Self {
        Self::of_seconds(LEASE_SECS)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Attempt {
    pub(crate) token: OperationToken,
    pub(crate) table: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum Taken {
    Attempt(Attempt),
    Stands,
}

impl Taken {
    #[cfg(test)]
    pub(crate) fn attempt(self) -> Attempt {
        match self {
            Self::Attempt(attempt) => attempt,
            Self::Stands => panic!("a dataset already stands under that name"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Finish {
    Provisioned(String),
    Ready,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owning {
    Held,
    Lost,
}

pub(crate) fn finishing(held: Option<&Dataset>, token: &OperationToken) -> Owning {
    let owns = held
        .and_then(|held| held.held.as_ref())
        .is_some_and(|held| held.token == *token);

    if owns { Owning::Held } else { Owning::Lost }
}

#[async_trait]
pub(crate) trait Datasets: Send + Sync + fmt::Debug {
    async fn get(&self, name: &DefinitionId) -> Result<Option<Dataset>, DatasetStoreError>;

    async fn list(&self) -> Result<Vec<String>, DatasetStoreError>;

    async fn page(&self, needle: &str, page: Page) -> Result<DatasetPage, DatasetStoreError>;

    async fn take_create(
        &self,
        name: &DefinitionId,
        declaration: &Value,
    ) -> Result<Taken, DatasetStoreError>;

    async fn take_remove(&self, name: &DefinitionId) -> Result<Attempt, DatasetStoreError>;

    async fn replace(
        &self,
        name: &DefinitionId,
        declaration: &Value,
    ) -> Result<bool, DatasetStoreError>;

    async fn finish(
        &self,
        name: &DefinitionId,
        token: &OperationToken,
        finish: Finish,
    ) -> Result<Owning, DatasetStoreError>;
}

#[derive(Debug, Error)]
pub(crate) enum DatasetStoreError {
    #[error(transparent)]
    Refused(#[from] Refused),
    #[error("dataset store operation failed")]
    Database(#[from] sea_orm::DbErr),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("`{0}` is not a state this service wrote")]
    UnreadableRow(String),
}

#[cfg(test)]
mod tests;
