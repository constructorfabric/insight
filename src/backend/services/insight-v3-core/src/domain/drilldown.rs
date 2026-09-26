//! The rows behind a widget: a stored metric, ordered and paged by the
//! reader rather than cut by the metric's own limit.
//!
//! The metric's compiler knows nothing of this. Its query becomes a subquery,
//! and the order, the page and the cursor sit outside it, so the same
//! wrapper serves a grouped metric and a plain one, a dataset and a table.

mod cursor;
mod order;
mod paged;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::definition::DefinitionName;
use super::metric_run::{MetricRuns, Source};
use super::query::metric_query::{ColumnKind, MetricRunner};
use super::query::time_window::WindowRequest;
use super::surfaces::CustomError;

pub(crate) use cursor::CursorError;

pub(crate) const DEFAULT_PAGE_ROWS: usize = 100;
pub(crate) const MAX_PAGE_ROWS: usize = 250;

/// What a reader asked one page for.
#[derive(Debug)]
pub(crate) struct Asked {
    pub(crate) window: WindowRequest,
    /// The window as it was written. A cursor is bound to the words, so a
    /// page asked for over another window is refused rather than served.
    pub(crate) range: Option<String>,
    pub(crate) bucket: Option<bool>,
    pub(crate) sort: Option<Sort>,
    pub(crate) limit: usize,
    pub(crate) cursor: Option<String>,
}

/// The column a page is ordered by, and which way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Sort {
    pub(crate) key: String,
    pub(crate) descending: bool,
}

/// One column of the page, as far as a reader ordering by or showing it
/// needs to know.
#[derive(Debug, Serialize)]
pub(crate) struct Column {
    pub(crate) key: String,
    pub(crate) kind: ColumnKind,
    /// Whether its numbers are percentages, which the metric says.
    pub(crate) percent: bool,
}

/// One page of a metric's rows, and how to ask for the next.
#[derive(Debug)]
pub(crate) struct Page {
    pub(crate) columns: Vec<Column>,
    pub(crate) rows: Vec<serde_json::Map<String, serde_json::Value>>,
    /// The order the page is in - always, so a reader that asked for none
    /// is told what it got.
    pub(crate) sort: Sort,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Error)]
pub(crate) enum DrilldownError {
    #[error(transparent)]
    Custom(CustomError),
    #[error("`{0}` is not a column of this metric, so its rows cannot be ordered by it")]
    Sort(String),
    #[error("a page holds between 1 and {MAX_PAGE_ROWS} rows")]
    PageSize,
    #[error(transparent)]
    Cursor(#[from] CursorError),
    #[error("the table behind this metric was made again while it was being read")]
    Rebuilt,
    #[error("`{0}` is a name this read keeps for itself; give that column another `as_name`")]
    Reserved(String),
}

/// The rows behind a metric, one page at a time.
#[derive(Debug)]
pub(crate) struct Drilldowns<'a> {
    runs: MetricRuns<'a>,
    runner: &'a MetricRunner,
}

impl<'a> Drilldowns<'a> {
    pub(crate) fn new(runs: MetricRuns<'a>, runner: &'a MetricRunner) -> Self {
        Self { runs, runner }
    }

    /// One page of the metric's rows, in the order asked for, over the
    /// window asked for; and the cursor that continues it, when there is
    /// more.
    pub(crate) async fn page(
        &self,
        name: &DefinitionName,
        asked: &Asked,
    ) -> Result<Page, DrilldownError> {
        if asked.limit == 0 || asked.limit > MAX_PAGE_ROWS {
            return Err(DrilldownError::PageSize);
        }

        // A continued walk reads the window its first page resolved, not
        // the one the same words would resolve to now.
        let resume = asked.cursor.as_deref().map(cursor::decode).transpose()?;
        let window = match &resume {
            Some(envelope) => envelope.window.clone(),
            None => MetricRuns::resolve(&asked.window).map_err(DrilldownError::Custom)?,
        };
        let prepared = self
            .runs
            .prepare(name, &window)
            .await
            .map_err(DrilldownError::Custom)?;
        let columns: Vec<Column> = prepared
            .kinds
            .iter()
            .map(|(key, kind)| Column {
                key: key.clone(),
                kind: *kind,
                percent: prepared.compiled.percents().contains(key),
            })
            .collect();
        if let Some(taken) = columns
            .iter()
            .find(|column| order::is_reserved(&column.key))
        {
            return Err(DrilldownError::Reserved(taken.key.clone()));
        }

        let sort = match &asked.sort {
            Some(sort) => sort.clone(),
            None => default_sort(prepared.own_order.as_ref(), &columns),
        };
        if !columns.iter().any(|column| column.key == sort.key) {
            return Err(DrilldownError::Sort(sort.key));
        }

        let snapshot = self.snapshot_of(&prepared.source).await?;
        let fingerprint = cursor::fingerprint(
            name.as_str(),
            &prepared.body,
            asked.range.as_deref(),
            asked.bucket,
            &window,
            &sort,
            &columns,
        );
        let order = order::OrderKey::build(&columns, &sort, prepared.compiled.hidden());

        let resume = resume
            .map(|envelope| resumed(envelope, &fingerprint, &snapshot, &order))
            .transpose()?;

        let (sql, binds) = paged::wrap(
            &prepared.compiled,
            &order,
            resume.as_ref(),
            asked.limit,
            &columns,
        );
        let mut rows = self
            .runner
            .page(&sql, &binds)
            .await
            .map_err(|error| DrilldownError::Custom(CustomError::Run(error)))?;

        let next_cursor = if rows.len() > asked.limit {
            rows.truncate(asked.limit);
            match rows.last() {
                Some(last) => Some(cursor::encode(
                    &fingerprint,
                    &snapshot,
                    &window,
                    order.key_of(last)?,
                )),
                None => None,
            }
        } else {
            None
        };
        let rows = rows
            .into_iter()
            .map(|row| order::presented(row, &columns, prepared.compiled.hidden()))
            .collect();

        Ok(Page {
            columns,
            rows,
            sort,
            next_cursor,
        })
    }
}

impl Drilldowns<'_> {
    /// What a cursor is bound to beside the selection: the table it was read
    /// from. A warehouse table rebuilt under the same name, or a dataset made
    /// again, is another table, and a position in the old one means nothing
    /// in the new.
    async fn snapshot_of(&self, source: &Source) -> Result<String, DrilldownError> {
        match source {
            Source::Table { database, table } => Ok(self
                .runner
                .table_uuid(database.as_deref(), table)
                .await
                .map_err(|error| DrilldownError::Custom(CustomError::Run(error)))?
                .unwrap_or_default()),
            Source::Dataset { table } => Ok(table.clone()),
        }
    }
}

/// The key a cursor holds, once the cursor has proved it was issued over
/// this very read.
fn resumed(
    envelope: cursor::Envelope,
    fingerprint: &str,
    snapshot: &str,
    order: &order::OrderKey,
) -> Result<cursor::CursorKey, DrilldownError> {
    if envelope.fingerprint != fingerprint {
        return Err(CursorError::Selection.into());
    }
    if envelope.snapshot != snapshot {
        return Err(DrilldownError::Rebuilt);
    }
    order.check(&envelope.key)?;

    Ok(envelope.key)
}

/// The order a page comes in when the reader named none: the metric's own
/// where it names a column, the first column upward otherwise.
fn default_sort(own: Option<&(String, bool)>, columns: &[Column]) -> Sort {
    if let Some((key, descending)) = own
        && columns.iter().any(|column| &column.key == key)
    {
        return Sort {
            key: key.clone(),
            descending: *descending,
        };
    }

    Sort {
        key: columns
            .first()
            .map(|column| column.key.clone())
            .unwrap_or_default(),
        descending: false,
    }
}

#[cfg(test)]
mod tests;
