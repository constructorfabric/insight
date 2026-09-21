//! Taking a record into a dataset.
//!
//! A record lands whole and unread: what the fields of it mean is the
//! declaration's business, and it says so at read time, not now.

use serde_json::Value;
use thiserror::Error;

use super::datasets::{DatasetStoreError, Datasets};
use super::definition::DefinitionName;
use super::kinds::dataset::declaration::{Declaration, Source};
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
            // Only here, so the ordinary path does not parse a declaration
            // for every record: a dataset with no table of ours is either
            // mid-create or one that takes no records at all, and the two are
            // different answers.
            return Err(takes_no_records(&dataset.declaration));
        };

        let body = serde_json::to_string(raw_data)?;

        match self.tables.insert(&table, name.as_str(), &body).await {
            Err(DatasetTableError::Vanished) => Err(IngestError::NotReady),
            other => Ok(other?),
        }
    }
}

/// Why a dataset holds no table of ours: it reads a relation the warehouse
/// builds, or it is still being made.
fn takes_no_records(declaration: &Value) -> IngestError {
    match serde_json::from_value::<Declaration>(declaration.clone()) {
        Ok(declaration) if !matches!(declaration.source, Source::Stream) => {
            IngestError::TakesNoRecords
        }
        Ok(_) | Err(_) => IngestError::NotReady,
    }
}

#[derive(Debug, Error)]
pub(crate) enum IngestError {
    #[error("this dataset reads a relation the warehouse builds; records are not sent into it")]
    TakesNoRecords,
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
