//! Looking at what a dataset holds: the latest records, as they arrived.

use super::datasets::{self, Datasets};
use super::definition::DefinitionName;
use crate::store::dataset_tables::{DatasetTableError, DatasetTables, Record};

/// Reads a dataset's own records, for a reader deciding whether what arrives
/// is what they meant to send.
#[derive(Debug)]
pub(crate) struct DatasetRecords<'a> {
    datasets: &'a dyn Datasets,
    tables: &'a DatasetTables,
    /// How many records one look shows, whatever the dataset holds.
    cap: u64,
}

impl<'a> DatasetRecords<'a> {
    pub(crate) fn new(datasets: &'a dyn Datasets, tables: &'a DatasetTables, cap: u64) -> Self {
        Self {
            datasets,
            tables,
            cap,
        }
    }

    /// The latest records of a dataset that is ready, newest first.
    ///
    /// A dataset mid-create or mid-removal answers nothing: there is no table
    /// to look in, or there is about to be none.
    pub(crate) async fn latest(&self, name: &DefinitionName) -> Result<Vec<Record>, PreviewError> {
        let ready = datasets::ready(self.datasets, name.as_str())
            .await
            .ok_or_else(|| PreviewError::NotReady(name.as_str().to_owned()))?;

        match self.tables.latest(&ready.table, self.cap).await {
            Ok(records) => Ok(records),
            // The dataset went while this read was in flight, which is the
            // same answer as its never having been there.
            Err(DatasetTableError::Vanished) => {
                Err(PreviewError::NotReady(name.as_str().to_owned()))
            }
            Err(error) => Err(PreviewError::Table(error)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PreviewError {
    #[error("no dataset named `{0}` is ready to be read")]
    NotReady(String),
    #[error(transparent)]
    Table(DatasetTableError),
}
