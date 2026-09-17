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
use super::definition::{DefinitionKind, DefinitionName, DefinitionStoreError, Definitions};
use super::kinds::dataset::declaration::Declaration;
use super::kinds::dataset::state::{DatasetState, Operation};
use super::kinds::dataset::validate::validate;
use super::kinds::metric;
use super::kinds::metric::answerable;
use super::query::metric_query::MetricQuery;
use super::violation::Violation;
use crate::store::dataset_tables::{DatasetTableError, DatasetTables, Shape};

/// Creating and removing datasets, which only an administrator does.
#[derive(Debug)]
pub(crate) struct DatasetLifecycle<'a> {
    datasets: &'a dyn Datasets,
    tables: &'a DatasetTables,
    definitions: &'a dyn Definitions,
}

impl<'a> DatasetLifecycle<'a> {
    pub(crate) fn new(
        datasets: &'a dyn Datasets,
        tables: &'a DatasetTables,
        definitions: &'a dyn Definitions,
    ) -> Self {
        Self {
            datasets,
            tables,
            definitions,
        }
    }

    /// Every stored metric reading this dataset, with the body that reads it.
    async fn readers(
        &self,
        name: &DefinitionName,
    ) -> Result<Vec<(String, Value)>, DatasetChangeError> {
        let mut reading = Vec::new();

        for held in self.definitions.list(DefinitionKind::Metric).await? {
            let Ok(parsed) = DefinitionName::parse(&held) else {
                continue;
            };
            let Some(body) = self
                .definitions
                .get(DefinitionKind::Metric, &parsed)
                .await?
            else {
                continue;
            };
            if metric::reads_dataset(&body).as_deref() == Some(name.as_str()) {
                reading.push((held, body));
            }
        }

        Ok(reading)
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
            return self.replace(name, body, &declaration).await;
        }

        self.bring_into_being(name, body).await
    }

    /// Whether a ready dataset already answers to this name.
    async fn stands(&self, name: &DefinitionName) -> Result<bool, DatasetChangeError> {
        let held = self.datasets.get(name).await?;

        Ok(held.is_some_and(|held| held.state == DatasetState::Ready))
    }

    /// The declaration of a dataset that stands is replaced where it lies,
    /// unless a metric reading it would break or quietly start answering
    /// something else.
    async fn replace(
        &self,
        name: &DefinitionName,
        body: &Value,
        after: &Declaration,
    ) -> Result<Value, DatasetChangeError> {
        let Some(held) = self.datasets.get(name).await? else {
            return Err(DatasetChangeError::NotFound);
        };
        let before: Declaration = read(&held.declaration)?;

        let broken = self.broken_by(name, &before, after).await?;
        if !broken.is_empty() {
            return Err(DatasetChangeError::WouldBreak(broken));
        }

        self.datasets.replace(name, body).await?;

        Ok(body.clone())
    }

    /// What this replacement would do to the metrics reading the dataset.
    ///
    /// Two of these leave a metric invalid, and two leave it valid while
    /// moving every number it has already answered, which a reader cannot see
    /// happen.
    async fn broken_by(
        &self,
        name: &DefinitionName,
        before: &Declaration,
        after: &Declaration,
    ) -> Result<Vec<Broken>, DatasetChangeError> {
        let identity_moved = before.row_identity != after.row_identity;
        let mut broken = Vec::new();

        for (reader, body) in self.readers(name).await? {
            let Ok(written) = serde_json::from_value::<MetricQuery>(body) else {
                continue;
            };

            for violation in answerable::check(&written, after) {
                broken.push(Broken::new(&reader, violation.detail));
            }

            if clock_of(&written, before) != clock_of(&written, after) {
                broken.push(Broken::new(
                    &reader,
                    "the date its window selects by would change".to_owned(),
                ));
            }

            for moved in fields_moved(&written, before, after) {
                broken.push(Broken::new(
                    &reader,
                    format!("`{moved}` would be read from somewhere else in the record"),
                ));
            }

            if identity_moved {
                broken.push(Broken::new(
                    &reader,
                    "which records count as one would change".to_owned(),
                ));
            }
        }

        Ok(broken)
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

        let readers: Vec<String> = self
            .readers(name)
            .await?
            .into_iter()
            .map(|(reader, _)| reader)
            .collect();
        if !readers.is_empty() {
            return Err(DatasetChangeError::StillRead(readers));
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

/// One metric a replacement would not leave as it found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Broken {
    pub(crate) metric: String,
    pub(crate) why: String,
}

impl Broken {
    fn new(metric: &str, why: String) -> Self {
        Self {
            metric: metric.to_owned(),
            why,
        }
    }
}

/// The field a metric's window would select by under this declaration.
fn clock_of<'a>(metric: &'a MetricQuery, declaration: &'a Declaration) -> Option<&'a str> {
    answerable::effective_clock(metric, declaration).map(|(field, _)| field)
}

/// The fields this metric reads that would come from a different place in the
/// record, leaving it valid and answering something else.
fn fields_moved(metric: &MetricQuery, before: &Declaration, after: &Declaration) -> Vec<String> {
    metric
        .field_references()
        .into_iter()
        .filter(|reference| {
            let was = before.field(reference.field).map(|field| &field.path);
            let now = after.field(reference.field).map(|field| &field.path);

            was.is_some() && now.is_some() && was != now
        })
        .map(|reference| reference.field.to_owned())
        .collect()
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
    #[error("{} still read this dataset", .0.join(", "))]
    StillRead(Vec<String>),
    #[error("the replacement would not leave every metric reading this dataset as it found it")]
    WouldBreak(Vec<Broken>),
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
    #[error(transparent)]
    Definitions(#[from] DefinitionStoreError),
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
