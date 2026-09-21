//! Taking a record into a dataset.
//!
//! A record lands whole and unread: what the fields of it mean is the
//! declaration's business, and it says so at read time, not now.

use serde_json::Value;
use thiserror::Error;

use super::datasets::{DatasetStoreError, Datasets};
use super::definition::DefinitionName;
use super::kinds::dataset::state::DatasetState;
use crate::store::dataset_tables::{DatasetTableError, DatasetTables};

/// Where records sent to a dataset go.
#[derive(Debug)]
pub(crate) struct DatasetIngest<'a> {
    datasets: &'a dyn Datasets,
    tables: &'a DatasetTables,
}

impl<'a> DatasetIngest<'a> {
    pub(crate) fn new(datasets: &'a dyn Datasets, tables: &'a DatasetTables) -> Self {
        Self { datasets, tables }
    }

    /// Stores one record in the dataset's table.
    ///
    /// The dataset is read without holding its row: a write per record cannot
    /// wait on a lock a removal might be holding. A removal that wins the race
    /// leaves no table, and the record is then refused rather than accepted —
    /// a record that reached no table was not stored.
    pub(crate) async fn receive(
        &self,
        name: &DefinitionName,
        raw_data: &Value,
    ) -> Result<(), IngestError> {
        let Some(dataset) = self.datasets.get(name).await? else {
            return Err(IngestError::NotReady);
        };

        if dataset.state != DatasetState::Ready {
            return Err(IngestError::NotReady);
        }

        let Some(table) = dataset.physical_table else {
            return Err(IngestError::NotReady);
        };

        let body = serde_json::to_string(raw_data)?;

        match self.tables.insert(&table, name.as_str(), &body).await {
            Err(DatasetTableError::Vanished) => Err(IngestError::NotReady),
            other => Ok(other?),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum IngestError {
    #[error("no dataset of that name is ready to take records")]
    NotReady,
    #[error("the record could not be read")]
    Unreadable(#[from] serde_json::Error),
    #[error(transparent)]
    Table(#[from] DatasetTableError),
    #[error(transparent)]
    Store(#[from] DatasetStoreError),
}

#[cfg(test)]
mod tests;
