use serde_json::Value;
use thiserror::Error;

use super::datasets::{
    Attempt, DatasetStoreError, Datasets, Finish, OperationToken, Owning, Refused, Taken,
};
use super::definition::{DefinitionId, DefinitionKind, DefinitionStoreError, Definitions};
use super::kinds::dataset::declaration::Declaration;
use super::kinds::dataset::shape;
use super::kinds::dataset::state::Operation;
use super::kinds::dataset::validate::validate;
use super::kinds::metric;
use super::kinds::metric::answerable;
use super::query::metric_query::MetricQuery;
use super::violation::Violation;
use crate::store::dataset_tables::{DatasetTableError, DatasetTables, Shape};

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

    async fn readers(
        &self,
        name: &DefinitionId,
    ) -> Result<Vec<(String, Value)>, DatasetChangeError> {
        let mut reading = Vec::new();

        for held in self.definitions.list(DefinitionKind::Metric).await? {
            let Ok(parsed) = DefinitionId::parse(&held) else {
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

    pub(crate) async fn dependents(
        &self,
        name: &DefinitionId,
    ) -> Result<Vec<String>, DatasetChangeError> {
        Ok(self
            .readers(name)
            .await?
            .into_iter()
            .map(|(reader, _)| reader)
            .collect())
    }

    pub(crate) async fn declare(
        &self,
        name: &DefinitionId,
        body: &Value,
    ) -> Result<Value, DatasetChangeError> {
        let violations = shape::check(body);
        if !violations.is_empty() {
            return Err(DatasetChangeError::Invalid(violations));
        }

        let declaration = read(body)?;
        let violations = validate(name.as_str(), &declaration);
        if !violations.is_empty() {
            return Err(DatasetChangeError::Invalid(violations));
        }

        match self.datasets.take_create(name, body).await {
            Ok(Taken::Stands) => self.replace(name, body, &declaration).await,
            Ok(Taken::Attempt(attempt)) => self.bring_into_being(name, body, attempt).await,
            Err(error) => {
                held_by_another(name, &error);

                Err(error.into())
            }
        }
    }

    async fn replace(
        &self,
        name: &DefinitionId,
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

        if !self.datasets.replace(name, body).await? {
            return Err(DatasetChangeError::NotFound);
        }

        Ok(body.clone())
    }

    async fn broken_by(
        &self,
        name: &DefinitionId,
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
        name: &DefinitionId,
        body: &Value,
        attempt: Attempt,
    ) -> Result<Value, DatasetChangeError> {
        let table = attempt.token.table(name);

        if let Err(error) = self.tables.provision(&table).await {
            match error {
                DatasetTableError::NotOurs(_) => {}
                DatasetTableError::Timeout => {
                    tracing::error!(
                        dataset = name.as_str(),
                        table,
                        "a dataset's table was left where the warehouse may still be making it"
                    );
                }
                DatasetTableError::Vanished
                | DatasetTableError::ClickHouse(_)
                | DatasetTableError::NotOurDatabase
                | DatasetTableError::DatabaseTopology(_) => self.discard(&table).await,
            }

            return Err(error.into());
        }

        match self.record(name, &attempt, table.clone()).await {
            Ok(Owning::Held) => Ok(body.clone()),
            Ok(Owning::Lost) => {
                finished_stale(name, Operation::Create);
                self.discard(&table).await;

                Err(DatasetChangeError::Refused(Refused::Busy(
                    Operation::Create,
                )))
            }
            Err(error) => {
                tracing::error!(
                    error = ?error,
                    dataset = name.as_str(),
                    table,
                    "a dataset was left with a table whose publication is unknown"
                );

                Err(error)
            }
        }
    }

    pub(crate) async fn remove(&self, name: &DefinitionId) -> Result<Removal, DatasetChangeError> {
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
            Err(DatasetStoreError::Refused(Refused::Busy(Operation::Remove))) => {
                return Ok(Removal::AlreadyUnderWay);
            }
            Err(DatasetStoreError::Refused(Refused::Gone)) => {
                return Err(DatasetChangeError::NotFound);
            }
            Err(other) => {
                held_by_another(name, &other);
                return Err(other.into());
            }
        };

        if let Err(error) = self.drop_records(attempt.table.as_deref()).await {
            tracing::error!(
                error = ?error,
                dataset = name.as_str(),
                "a dataset's table could not be dropped"
            );

            return Err(error);
        }

        match self
            .datasets
            .finish(name, &attempt.token, Finish::Removed)
            .await?
        {
            Owning::Held => Ok(Removal::Removed),
            Owning::Lost => {
                finished_stale(name, Operation::Remove);

                Err(DatasetChangeError::Refused(Refused::Busy(
                    Operation::Remove,
                )))
            }
        }
    }

    async fn drop_records(&self, table: Option<&str>) -> Result<(), DatasetChangeError> {
        let Some(table) = table else {
            return Ok(());
        };

        match self.tables.shape_of(table).await? {
            Shape::Absent => Ok(()),
            Shape::Ingest => Ok(self.tables.drop_table(table).await?),
            Shape::Foreign => Err(DatasetChangeError::Table(DatasetTableError::NotOurs(
                table.to_owned(),
            ))),
        }
    }

    async fn record(
        &self,
        name: &DefinitionId,
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

    async fn discard(&self, table: &str) {
        if let Err(error) = self.tables.drop_table(table).await {
            tracing::error!(error = ?error, table, "a table this attempt lost could not be dropped");
        }
    }
}

fn read(body: &Value) -> Result<Declaration, DatasetChangeError> {
    serde_json::from_value(body.clone()).map_err(DatasetChangeError::Unreadable)
}

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

fn clock_of<'a>(metric: &'a MetricQuery, declaration: &'a Declaration) -> Option<&'a str> {
    answerable::effective_clock(metric, declaration).map(|(field, _)| field)
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Removal {
    Removed,
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

fn held_by_another(name: &DefinitionId, error: &DatasetStoreError) {
    if let DatasetStoreError::Refused(Refused::Busy(operation)) = error {
        tracing::warn!(
            dataset = name.as_str(),
            holder = operation.as_str(),
            "a dataset is held by another attempt"
        );
    }
}

fn finished_stale(name: &DefinitionId, operation: Operation) {
    tracing::warn!(
        dataset = name.as_str(),
        operation = operation.as_str(),
        "an attempt finished after its lease had lapsed"
    );
}

#[cfg(test)]
mod tests;
