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
use super::kinds::dataset::state::DatasetState;
use super::kinds::dataset::validate::{Violation, validate};
use crate::store::dataset_tables::{DatasetTableError, DatasetTables};

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
                    super::kinds::dataset::state::Operation::Create,
                )))
            }
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

#[derive(Debug, Error)]
pub(crate) enum DatasetChangeError {
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
