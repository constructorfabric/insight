//! Datasets as they are held: the row, and who is allowed to move it.
//!
//! A dataset owns a table of records, so changing one means a warehouse call,
//! and no row is held across that. An attempt takes a leased operation
//! instead: it alone may touch that dataset's tables, and it writes its
//! outcome only while it still owns the operation.

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::Value;
use thiserror::Error;

use super::definition::DefinitionName;
use super::kinds::dataset::lifecycle::{DatasetState, Operation};

/// How long an attempt owns a dataset before another may take over.
///
/// Long enough for a table to be created or dropped, short enough that an
/// abandoned attempt does not hold a name for a shift.
pub(crate) const LEASE_SECS: i64 = 60;

/// The prefix every table this service provisions carries, so a table of its
/// own is told from anything else in the datasets database at a glance.
const TABLE_PREFIX: &str = "ds_";

/// Proof that an attempt still owns the operation it took.
///
/// INVARIANT: the table a create provisions is named after the token, never
/// after the dataset, so a drop arriving from an attempt that has since lost
/// the dataset names a table no later attempt uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperationToken(String);

impl OperationToken {
    pub(crate) fn mint() -> Self {
        Self(uuid::Uuid::now_v7().simple().to_string())
    }

    /// The token as a row recorded it.
    pub(crate) fn from_row(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The table a create under this token provisions.
    pub(crate) fn table(&self) -> String {
        format!("{TABLE_PREFIX}{}", self.0)
    }
}

/// The operation an attempt holds a dataset for, and until when.
#[derive(Debug, Clone)]
pub(crate) struct Held {
    pub(crate) operation: Operation,
    pub(crate) token: OperationToken,
    pub(crate) until: DateTime<Utc>,
}

/// A dataset as its row records it.
#[derive(Debug, Clone)]
pub(crate) struct Dataset {
    pub(crate) name: DefinitionName,
    pub(crate) declaration: Value,
    pub(crate) state: DatasetState,
    /// The table holding its records, absent until one is provisioned.
    pub(crate) physical_table: Option<String>,
    pub(crate) held: Option<Held>,
}

impl Dataset {
    /// The operation holding this dataset, if one still is.
    ///
    /// A lapsed lease never means the attempt succeeded: it means its outcome
    /// no longer counts, which finishing an operation enforces.
    pub(crate) fn holder(&self, now: DateTime<Utc>) -> Option<Operation> {
        self.held
            .as_ref()
            .filter(|held| held.until > now)
            .map(|held| held.operation)
    }
}

/// Why an attempt may not take a dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum Refused {
    #[error("a {0} of this dataset is already under way")]
    Busy(Operation),
    #[error("there is no such dataset")]
    Gone,
    #[error("this name belongs to a removal until it finishes")]
    Removing,
}

/// What taking an operation comes to, read off the row alone.
///
/// A pure decision so that the store and the one the tests use cannot come to
/// different answers about the same row.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Taking {
    /// No row yet: the insert claims the name, and loses to whoever inserted
    /// first, because the name is the key.
    Claim,
    /// There is nothing to remove.
    Gone,
    /// A row this attempt may take, into the state the operation implies.
    Take(DatasetState),
    Refuse(Refused),
}

/// Whether `operation` may be taken on the dataset `held` describes.
///
/// A ready dataset is taken for a create by the flow that replaces one; a
/// replacement that changes no table never comes here at all.
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

    Taking::Take(match operation {
        Operation::Create => DatasetState::Claimed,
        Operation::Remove => DatasetState::Removing,
    })
}

/// When a lease taken now lapses.
pub(crate) fn lease_until(now: DateTime<Utc>) -> DateTime<Utc> {
    now + TimeDelta::seconds(LEASE_SECS)
}

/// What an attempt carries away from taking an operation.
#[derive(Debug, Clone)]
pub(crate) struct Attempt {
    pub(crate) token: OperationToken,
    pub(crate) state: DatasetState,
}

/// Where dataset rows are kept.
#[async_trait]
pub(crate) trait Datasets: Send + Sync + fmt::Debug {
    /// The dataset as it stands, or nothing if the name is free.
    async fn get(&self, name: &DefinitionName) -> Result<Option<Dataset>, DatasetStoreError>;

    /// Every dataset name, whatever state it is in.
    async fn list(&self) -> Result<Vec<String>, DatasetStoreError>;

    /// Takes `operation` on the dataset for a fresh attempt, holding the row
    /// for the whole decision.
    async fn take_operation(
        &self,
        name: &DefinitionName,
        operation: Operation,
        declaration: &Value,
    ) -> Result<Attempt, DatasetStoreError>;
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
