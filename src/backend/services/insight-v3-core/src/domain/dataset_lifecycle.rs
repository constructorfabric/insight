//! Bringing a dataset into being, and what that costs when it goes wrong.
//!
//! A dataset is a declaration and a table of records, and the two are written
//! by different systems. An attempt owns the dataset while it works, and
//! publishes only while it still owns it; whatever it made and then lost, it
//! takes away again.

use serde_json::Value;
use thiserror::Error;

use super::datasets::{
    Attempt, DatasetStoreError, Datasets, Finish, OperationToken, Owning, Refused,
};
use super::definition::DefinitionName;
use super::kinds::dataset::declaration::Declaration;
use super::kinds::dataset::state::{DatasetState, Operation};
use super::kinds::dataset::validate::{Violation, validate};
use crate::store::dataset_tables::{DatasetTableError, DatasetTables, Shape};

/// Creating and removing datasets, which only an administrator does.
#[derive(Debug)]
pub(crate) struct DatasetLifecycle<'a> {
    datasets: &'a dyn Datasets,
    tables: &'a DatasetTables,
}

impl<'a> DatasetLifecycle<'a> {
    pub(crate) fn new(datasets: &'a dyn Datasets, tables: &'a DatasetTables) -> Self {
        Self { datasets, tables }
    }

    /// Declares a dataset, or replaces the declaration of one that stands.
    ///
    /// A replacement of a ready dataset touches no table, so it commits in one
    /// transaction and nothing can interleave with it. Bringing a new one into
    /// being needs a table, and therefore an operation that outlives a
    /// transaction.
    pub(crate) async fn declare(
        &self,
        name: &DefinitionName,
        body: &Value,
    ) -> Result<Value, DatasetChangeError> {
        let declaration = read(body)?;
        let violations = validate(name.as_str(), &declaration);
        if !violations.is_empty() {
            return Err(DatasetChangeError::Invalid(violations));
        }

        if self.stands(name).await? {
            return self.replace(name, body).await;
        }

        self.bring_into_being(name, body).await
    }

    /// Whether a ready dataset already answers to this name.
    async fn stands(&self, name: &DefinitionName) -> Result<bool, DatasetChangeError> {
        let held = self.datasets.get(name).await?;

        Ok(held.is_some_and(|held| held.state == DatasetState::Ready))
    }

    /// The declaration of a dataset that stands is replaced where it lies.
    async fn replace(
        &self,
        name: &DefinitionName,
        body: &Value,
    ) -> Result<Value, DatasetChangeError> {
        self.datasets.replace(name, body).await?;

        Ok(body.clone())
    }

    async fn bring_into_being(
        &self,
        name: &DefinitionName,
        body: &Value,
    ) -> Result<Value, DatasetChangeError> {
        let attempt = self.datasets.take_create(name, body).await?;
        let table = attempt.token.table(name);

        self.tables.provision(&table).await?;

        match self.record(name, &attempt, table.clone()).await? {
            Owning::Held => Ok(body.clone()),
            Owning::Lost => {
                self.discard(&table).await;

                Err(DatasetChangeError::Refused(Refused::Busy(
                    Operation::Create,
                )))
            }
        }
    }

    /// Takes a dataset away, with the records it holds.
    ///
    /// The table dropped is the one the row names and no other, so a drop
    /// arriving late cannot reach a table some later dataset provisioned.
    pub(crate) async fn remove(
        &self,
        name: &DefinitionName,
    ) -> Result<Removal, DatasetChangeError> {
        if self.datasets.get(name).await?.is_none() {
            return Err(DatasetChangeError::NotFound);
        }

        let attempt = match self.datasets.take_remove(name).await {
            Ok(attempt) => attempt,
            // A removal already under way is this request's own outcome
            // arriving by another route, not a conflict to report.
            Err(DatasetStoreError::Refused(Refused::Busy(Operation::Remove))) => {
                return Ok(Removal::AlreadyUnderWay);
            }
            Err(DatasetStoreError::Refused(Refused::Gone)) => {
                return Err(DatasetChangeError::NotFound);
            }
            Err(other) => return Err(other.into()),
        };

        self.drop_records(name).await?;

        match self
            .datasets
            .finish(name, &attempt.token, Finish::Removed)
            .await?
        {
            Owning::Held => Ok(Removal::Removed),
            Owning::Lost => Err(DatasetChangeError::Refused(Refused::Busy(
                Operation::Remove,
            ))),
        }
    }

    /// Drops the table this dataset's row names, if it names one and it is
    /// still a table of this service's own shape.
    async fn drop_records(&self, name: &DefinitionName) -> Result<(), DatasetChangeError> {
        let Some(table) = self
            .datasets
            .get(name)
            .await?
            .and_then(|held| held.physical_table)
        else {
            return Ok(());
        };

        match self.tables.shape_of(&table).await? {
            Shape::Absent => Ok(()),
            Shape::Ingest => Ok(self.tables.drop_table(&table).await?),
            // Something else holds the name now. Dropping it would take a
            // table this service never made.
            Shape::Foreign => Err(DatasetChangeError::Table(DatasetTableError::NotOurs(table))),
        }
    }

    /// Records the table and publishes the dataset, both only while this
    /// attempt still owns it.
    async fn record(
        &self,
        name: &DefinitionName,
        attempt: &Attempt,
        table: String,
    ) -> Result<Owning, DatasetChangeError> {
        let token: &OperationToken = &attempt.token;

        if self
            .datasets
            .finish(name, token, Finish::Provisioned(table))
            .await?
            == Owning::Lost
        {
            return Ok(Owning::Lost);
        }

        Ok(self.datasets.finish(name, token, Finish::Ready).await?)
    }

    /// Takes away a table this attempt made and then lost the right to.
    ///
    /// Nothing points at it: the declaration that would name it was never
    /// written. A drop that fails leaves a table no reader can reach, which is
    /// worth a line in the log and nothing more.
    async fn discard(&self, table: &str) {
        if let Err(error) = self.tables.drop_table(table).await {
            tracing::error!(error = ?error, table, "a table this attempt lost could not be dropped");
        }
    }
}

/// The body as a declaration, or the one violation that it is not one.
fn read(body: &Value) -> Result<Declaration, DatasetChangeError> {
    serde_json::from_value(body.clone()).map_err(DatasetChangeError::Unreadable)
}

/// What became of a removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Removal {
    Removed,
    /// Another attempt is already taking this dataset away.
    AlreadyUnderWay,
}

#[derive(Debug, Error)]
pub(crate) enum DatasetChangeError {
    #[error("there is no dataset of that name")]
    NotFound,
    #[error("the declaration is not valid")]
    Invalid(Vec<Violation>),
    #[error("the body is not a declaration: {0}")]
    Unreadable(serde_json::Error),
    #[error(transparent)]
    Refused(Refused),
    #[error(transparent)]
    Table(#[from] DatasetTableError),
    #[error(transparent)]
    Store(DatasetStoreError),
}

impl From<DatasetStoreError> for DatasetChangeError {
    fn from(error: DatasetStoreError) -> Self {
        match error {
            DatasetStoreError::Refused(refusal) => Self::Refused(refusal),
            other => Self::Store(other),
        }
    }
}

#[cfg(test)]
mod tests;
