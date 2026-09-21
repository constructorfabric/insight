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
use super::kinds::dataset::declaration::{Declaration, Source};
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
    /// INVARIANT: a lapsed lease is not a finished operation. Taking one over
    /// mints a fresh token, which is what makes the lapsed attempt's own
    /// finish a no-op.
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
    pub(crate) reads: Reads,
}

/// Where a ready dataset's rows are read from.
///
/// INVARIANT: a dataset over a stream is ready only once its table exists,
/// because there is nowhere to read until this service makes one. A dataset
/// over a relation is ready as soon as it is published: the relation is
/// already there, and this service neither made it nor may take it away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reads {
    /// A table this service made. Its database is the service's own, so the
    /// row records only which table.
    Ours(String),
    /// A relation the warehouse builds, which the declaration names whole.
    Warehouse { database: String, table: String },
}

impl Reads {
    /// The database and relation to read, given the one this service keeps
    /// its own tables in.
    pub(crate) fn at<'a>(&'a self, ours: &'a str) -> (&'a str, &'a str) {
        match self {
            Self::Ours(table) => (ours, table),
            Self::Warehouse { database, table } => (database, table),
        }
    }
}

/// The dataset under this name, when one is ready to be read.
///
/// Absent, still being made and being removed all read as none: the table is
/// not there yet, or is about to go. Every caller answers those the same way,
/// so none of them tells the three apart. A store that did not answer is not
/// one of them - it is ours, and it keeps its error.
pub(crate) async fn ready(
    datasets: &dyn Datasets,
    named: &str,
) -> Result<Option<Ready>, DatasetStoreError> {
    let Ok(name) = DefinitionName::parse(named) else {
        return Ok(None);
    };
    let Some(held) = datasets.get(&name).await? else {
        return Ok(None);
    };
    if held.state != DatasetState::Ready {
        return Ok(None);
    }
    let declaration: Declaration = serde_json::from_value(held.declaration)?;
    let reads = match &declaration.source {
        Source::Stream => match held.physical_table {
            Some(table) => Reads::Ours(table),
            // A create that has not provisioned yet: the row stands, the
            // table does not, and there is nothing to read.
            None => return Ok(None),
        },
        Source::Relation { database, table } => Reads::Warehouse {
            database: database.clone(),
            table: table.clone(),
        },
    };

    Ok(Some(Ready { declaration, reads }))
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
    /// A dataset already answers to this name. Its records stay where they
    /// are, so what follows is a replacement and not an operation at all.
    Stands,
    /// A row this attempt may take, into the state the operation implies.
    Take(DatasetState),
    Refuse(Refused),
}

/// Whether `operation` may be taken on the dataset `held` describes.
///
/// INVARIANT: a create never takes a dataset that stands. Taking it would
/// demote it out of sight, provision a second table and leave the records in
/// the first, so the answer is that it stands and the caller replaces it.
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
    /// The table the row named when this attempt took the operation.
    ///
    /// INVARIANT: a removal drops this table and no other. Reading the name
    /// again when the drop runs would read whatever the row says then, and a
    /// removal that outlived its lease would take the table of the dataset
    /// that has since been made under the same name.
    pub(crate) table: Option<String>,
}

/// What asking for a create came to.
#[derive(Debug, Clone)]
pub(crate) enum Taken {
    /// Nobody held the name, so this attempt owns it.
    Attempt(Attempt),
    /// A dataset already stands under it, and is replaced where it lies.
    Stands,
}

impl Taken {
    /// The attempt a free name hands over.
    #[cfg(test)]
    pub(crate) fn attempt(self) -> Attempt {
        match self {
            Self::Attempt(attempt) => attempt,
            Self::Stands => panic!("a dataset already stands under that name"),
        }
    }
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
///
/// The token alone answers it: a lease that lapsed is taken over by minting a
/// fresh token, so a row that still carries this one was never taken over.
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
    ) -> Result<Taken, DatasetStoreError>;

    /// Takes a removal for a fresh attempt.
    async fn take_remove(&self, name: &DefinitionName) -> Result<Attempt, DatasetStoreError>;

    /// Replaces the declaration of a dataset that stands, in one transaction:
    /// no table changes, so nothing may interleave with it. Answers whether
    /// there was still one standing to replace.
    async fn replace(
        &self,
        name: &DefinitionName,
        declaration: &Value,
    ) -> Result<bool, DatasetStoreError>;

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
