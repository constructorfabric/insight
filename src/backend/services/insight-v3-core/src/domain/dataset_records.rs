//! Looking at what a dataset holds: the latest records, as they arrived.

use super::datasets::{self, Datasets, Reads};
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

    /// The latest records of a dataset that is ready, newest first, and how
    /// many it holds in all.
    ///
    /// A dataset mid-create or mid-removal answers nothing: there is no table
    /// to look in, or there is about to be none.
    pub(crate) async fn latest(
        &self,
        name: &DefinitionName,
        wanted: Option<u64>,
    ) -> Result<Preview, PreviewError> {
        let ready = datasets::ready(self.datasets, name.as_str())
            .await
            .map_err(PreviewError::Store)?
            .ok_or_else(|| PreviewError::NotReady(name.as_str().to_owned()))?;

        // This reads through the connection bound to the datasets database,
        // which is the only one that may write, and it reads a record whole
        // out of its payload. A relation the warehouse builds has neither a
        // payload nor an arrival instant, so it is shown by its declared
        // columns instead - which is not this read.
        let Reads::Ours(ours) = ready.reads else {
            return Err(PreviewError::NotOurs(name.as_str().to_owned()));
        };

        let looked = async {
            let records = self.tables.latest(&ours, shown(wanted, self.cap)).await?;
            let total = self.tables.count(&ours).await?;
            Ok::<_, DatasetTableError>(Preview { records, total })
        };

        match looked.await {
            Ok(preview) => Ok(preview),
            // The dataset went while this read was in flight, which is the
            // same answer as its never having been there.
            Err(DatasetTableError::Vanished) => {
                Err(PreviewError::NotReady(name.as_str().to_owned()))
            }
            Err(error) => Err(PreviewError::Table(error)),
        }
    }
}

/// What one look at a dataset shows: a few of its records, and how many there are.
#[derive(Debug)]
pub(crate) struct Preview {
    pub(crate) records: Vec<Record>,
    pub(crate) total: u64,
}

/// How many records a look shows: what was asked for, within the cap; the
/// cap when nothing was.
fn shown(wanted: Option<u64>, cap: u64) -> u64 {
    wanted.map_or(cap, |asked| asked.clamp(1, cap))
}

#[cfg(test)]
mod tests {
    use super::shown;

    #[test]
    fn a_look_shows_what_was_asked_for_within_the_cap() {
        let cases = [(None, 50), (Some(20), 20), (Some(500), 50), (Some(0), 1)];

        for (wanted, expected) in cases {
            assert_eq!(shown(wanted, 50), expected, "wanted {wanted:?}");
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PreviewError {
    #[error("no dataset named `{0}` is ready to be read")]
    NotReady(String),
    #[error("`{0}` reads a relation the warehouse builds, whose records are not shown here yet")]
    NotOurs(String),
    #[error(transparent)]
    Store(crate::domain::datasets::DatasetStoreError),
    #[error(transparent)]
    Table(DatasetTableError),
}
