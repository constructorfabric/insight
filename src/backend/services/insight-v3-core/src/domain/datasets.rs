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

use super::definition::{DefinitionName, NamePage, Page};
use super::kinds::dataset::declaration::Declaration;
use super::kinds::dataset::state::{DatasetState, Operation};

/// How long an attempt owns a dataset before another may take over, where an
/// installation sets nothing.
///
/// Long enough for a table to be created or dropped, short enough that an
/// abandoned attempt does not hold a name for a shift.
pub(crate) const LEASE_SECS: i64 = 60;

/// The prefix every table this service provisions carries, so a table of its
/// own is told from anything else in the datasets database at a glance.
const TABLE_PREFIX: &str = "ds_";
/// How much of a dataset's name a physical name carries. The generation after
/// it is what makes the name unique, so this only has to stay readable.
const TABLE_NAME_CHARS: usize = 64;

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
    ///
    /// It carries the dataset's name so a reader recognises it, and this
    /// attempt's generation so that two attempts at one dataset never address
    /// one table.
    pub(crate) fn table(&self, dataset: &DefinitionName) -> String {
        let readable: String = dataset.as_str().chars().take(TABLE_NAME_CHARS).collect();

        format!("{TABLE_PREFIX}{readable}_{}", self.0)
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

/// A dataset that stands ready to be read: what its records mean, and where
/// they are kept.
#[derive(Debug)]
pub(crate) struct Ready {
    pub(crate) declaration: Declaration,
    pub(crate) table: String,
}

/// The dataset under this name, when one is ready to be read.
///
/// Absent, still being made and being removed all read as none: the table is
/// not there yet, or is about to go. Every caller answers that the same way,
/// so none of them tells the three apart.
pub(crate) async fn ready(datasets: &dyn Datasets, named: &str) -> Option<Ready> {
    let name = DefinitionName::parse(named).ok()?;
    let held = datasets.get(&name).await.ok()??;
    if held.state != DatasetState::Ready {
        return None;
    }
    let table = held.physical_table?;

    match serde_json::from_value(held.declaration) {
        Ok(declaration) => Some(Ready { declaration, table }),
        Err(error) => {
            tracing::error!(error = ?error, "a stored dataset declaration could not be read");
            None
        }
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

/// How long this installation lets an attempt hold a dataset.
///
/// An installation that cannot wait out an abandoned create shortens it; one
/// whose warehouse is slow to make a table lengthens it.
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

/// What an attempt carries away from taking an operation.
#[derive(Debug, Clone)]
pub(crate) struct Attempt {
    pub(crate) token: OperationToken,
}

/// What an attempt writes when its work is done, or part of it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Finish {
    /// The table this attempt provisioned. Recorded before the create is
    /// finished, so an attempt that has since lost the dataset learns to drop
    /// what it made.
    Provisioned(String),
    /// The dataset is ready: its declaration stands, its records have a home,
    /// and the operation is released.
    Ready,
    /// The dataset is gone, row and all.
    Removed,
}

/// Whether an attempt still owned the dataset when it came to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owning {
    Held,
    /// The dataset moved on while this attempt was working, so its outcome no
    /// longer counts and is discarded rather than published.
    Lost,
}

/// Whether the row still records this attempt as the owner.
pub(crate) fn finishing(held: Option<&Dataset>, token: &OperationToken) -> Owning {
    let owns = held
        .and_then(|held| held.held.as_ref())
        .is_some_and(|held| held.token == *token);

    if owns { Owning::Held } else { Owning::Lost }
}

/// Where dataset rows are kept.
#[async_trait]
pub(crate) trait Datasets: Send + Sync + fmt::Debug {
    /// The dataset as it stands, or nothing if the name is free.
    async fn get(&self, name: &DefinitionName) -> Result<Option<Dataset>, DatasetStoreError>;

    /// Every dataset name, whatever state it is in.
    async fn list(&self) -> Result<Vec<String>, DatasetStoreError>;

    /// One page of the datasets a reader may see: the ready ones matching
    /// `needle` over name and declaration, with how many match in all.
    ///
    /// A dataset mid-create or mid-removal is left out rather than shown in a
    /// state nothing can be done with.
    async fn page(&self, needle: &str, page: Page) -> Result<NamePage, DatasetStoreError>;

    /// Takes a create for a fresh attempt, holding the row for the whole
    /// decision. The declaration is the one this attempt means to publish, so
    /// taking over an abandoned create does not publish the abandoned body.
    async fn take_create(
        &self,
        name: &DefinitionName,
        declaration: &Value,
    ) -> Result<Attempt, DatasetStoreError>;

    /// Takes a removal for a fresh attempt.
    async fn take_remove(&self, name: &DefinitionName) -> Result<Attempt, DatasetStoreError>;

    /// Replaces the declaration of a dataset that stands, in one transaction:
    /// no table changes, so nothing may interleave with it.
    async fn replace(
        &self,
        name: &DefinitionName,
        declaration: &Value,
    ) -> Result<(), DatasetStoreError>;

    /// Writes what this attempt came to write, but only while it still owns
    /// the dataset.
    async fn finish(
        &self,
        name: &DefinitionName,
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
