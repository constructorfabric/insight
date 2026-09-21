//! Looking at what a dataset holds: the latest records, as they arrived.

use super::datasets::{self, Datasets};
use super::definition::DefinitionName;
use super::kinds::dataset::declaration::Declaration;
use super::kinds::dataset::read::{Form, PAYLOAD_COLUMN, read};
use crate::store::dataset_tables::{DatasetTableError, DatasetTables, Page, Record};

/// The column every record carries, whatever the dataset declares.
pub(crate) const ARRIVED: &str = "received_at";

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

    /// One page of a dataset's records, and how many it holds in all.
    ///
    /// A dataset mid-create or mid-removal answers nothing: there is no table
    /// to look in, or there is about to be none.
    pub(crate) async fn page(
        &self,
        name: &DefinitionName,
        look: &Look,
    ) -> Result<Preview, PreviewError> {
        let ready = datasets::ready(self.datasets, name.as_str())
            .await
            .map_err(PreviewError::Store)?
            .ok_or_else(|| PreviewError::NotReady(name.as_str().to_owned()))?;

        let order = ordering(&ready.declaration, look)?;
        let page = Page {
            limit: shown(look.limit, self.cap),
            offset: look.offset,
            order: &order,
        };

        let looked = async {
            let records = self.tables.page(&ready.table, page).await?;
            let total = self.tables.count(&ready.table).await?;
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

/// What a reader asked one look for.
#[derive(Debug, Default)]
pub(crate) struct Look {
    pub(crate) limit: Option<u64>,
    pub(crate) offset: u64,
    /// A declared field, or `received_at`. Absent means the order records
    /// arrived in.
    pub(crate) order_by: Option<String>,
    pub(crate) descending: bool,
}

/// How the page is ordered: by a declared field, read as the declaration says
/// it is read, or by the instant the record arrived.
fn ordering(declaration: &Declaration, look: &Look) -> Result<String, PreviewError> {
    let direction = if look.descending { "DESC" } else { "ASC" };
    let Some(named) = look.order_by.as_deref() else {
        return Ok(format!("{ARRIVED} DESC"));
    };
    if named == ARRIVED {
        return Ok(format!("{ARRIVED} {direction}"));
    }

    let field = declaration
        .fields
        .iter()
        .find(|field| field.name == named)
        .ok_or_else(|| PreviewError::NoSuchField {
            named: named.to_owned(),
            declared: declaration
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect(),
        })?;

    Ok(format!(
        "{} {direction}",
        read(field, Form::Raw, PAYLOAD_COLUMN)
    ))
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
    #[error("`{named}` is not a field of this dataset; it declares {}", declared.join(", "))]
    NoSuchField {
        named: String,
        declared: Vec<String>,
    },
    #[error("no dataset named `{0}` is ready to be read")]
    NotReady(String),
    #[error(transparent)]
    Store(crate::domain::datasets::DatasetStoreError),
    #[error(transparent)]
    Table(DatasetTableError),
}
