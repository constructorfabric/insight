//! Running a stored metric over a window the reader picked.

use chrono::Utc;

use super::datasets::{self, Datasets};
use super::definition::{DefinitionKind, DefinitionName, Lookup};
use super::kinds::dataset::declaration::Declaration;
use super::kinds::metric::answerable::EffectiveClock;
use super::query::metric_query::over::Over;
use super::query::metric_query::{MetricQuery, MetricQueryError, MetricRunner, RunResult};
use super::query::time_window::{Window, WindowRequest};
use super::query::undated::UndatedCount;
use super::surfaces::CustomError;
use crate::store::catalog::{Catalog, TableEngine};

/// Everything a metric needs to answer: the definition it is stored as, the
/// warehouse that holds the table, and the catalogue that says how.
#[derive(Debug)]
pub(crate) struct MetricRuns<'a> {
    definitions: &'a dyn Lookup,
    metrics: &'a MetricRunner,
    catalog: &'a Catalog,
    datasets: &'a dyn Datasets,
    /// The database every dataset's records are kept in.
    datasets_database: &'a str,
}

impl<'a> MetricRuns<'a> {
    pub(crate) fn new(
        definitions: &'a dyn Lookup,
        metrics: &'a MetricRunner,
        catalog: &'a Catalog,
        datasets: &'a dyn Datasets,
        datasets_database: &'a str,
    ) -> Self {
        Self {
            definitions,
            metrics,
            catalog,
            datasets,
            datasets_database,
        }
    }

    /// The stored body of a metric, or the refusal that there is none.
    async fn body_of(&self, name: &DefinitionName) -> Result<serde_json::Value, CustomError> {
        self.definitions
            .get(DefinitionKind::Metric, name)
            .await
            .map_err(CustomError::Store)?
            .ok_or_else(|| CustomError::NotFound {
                kind: DefinitionKind::Metric,
                name: name.as_str().to_owned(),
            })
    }

    /// Runs a stored metric over the window the caller asked for, resolved
    /// against the wall clock, so a named range means the period it is named
    /// after whether or not the data reaches that far.
    pub(crate) async fn run(
        &self,
        name: &DefinitionName,
        request: &WindowRequest,
    ) -> Result<RunResult, CustomError> {
        let body = self.body_of(name).await?;

        let metric: MetricQuery = serde_json::from_value(body).map_err(CustomError::Body)?;

        if let Some(named) = metric.dataset() {
            return self.run_over_dataset(&metric, named, request).await;
        }

        metric.check().map_err(CustomError::Compile)?;

        if request.is_ranged() && !metric.has_clock().map_err(CustomError::Compile)? {
            return Err(CustomError::Compile(MetricQueryError::ClocklessWindow));
        }

        let engine = self.engine_of(&metric).await?;
        let undated = self.undated_of(&metric, request, engine).await?;
        let window = request
            .resolve(Utc::now())
            .map_err(|error| CustomError::Compile(error.into()))?;

        let compiled = metric
            .compile_window(self.metrics.people(), &window, engine, None)
            .map_err(CustomError::Compile)?;

        let mut result = self
            .metrics
            .run(&compiled)
            .await
            .map_err(CustomError::Run)?;
        if request.is_ranged() {
            result.undated = Some(undated.count());
        }

        Ok(result)
    }

    /// Runs a metric over the dataset it reads.
    ///
    /// The relation, where each value sits and which records count as one all
    /// come from the declaration, so the metric contributes the question and
    /// the dataset contributes everything about the records.
    async fn run_over_dataset(
        &self,
        metric: &MetricQuery,
        named: &str,
        request: &WindowRequest,
    ) -> Result<RunResult, CustomError> {
        let (declaration, table) = self.ready_dataset(named).await?;
        let over = Over {
            declaration: &declaration,
            database: self.datasets_database,
            table: &table,
        };

        let window = request
            .resolve(Utc::now())
            .map_err(|error| CustomError::Compile(error.into()))?;
        let compiled = metric
            .compile_over(self.metrics.people(), &window, over)
            .map_err(CustomError::Compile)?;

        let mut result = self
            .metrics
            .run(&compiled)
            .await
            .map_err(CustomError::Run)?;
        result.clock = EffectiveClock::of(metric, &declaration);
        if request.is_ranged() {
            result.undated = Some(self.undated_over(metric, over).await?.count());
        }

        Ok(result)
    }

    /// Runs a metric nobody stored: the assistant's own, written to answer one
    /// question and kept nowhere.
    pub(crate) async fn answer(&self, metric: &MetricQuery) -> Result<RunResult, CustomError> {
        let Some(named) = metric.dataset() else {
            return Err(CustomError::Compile(MetricQueryError::NoDataset));
        };
        let (declaration, table) = self.ready_dataset(named).await?;
        let over = Over {
            declaration: &declaration,
            database: self.datasets_database,
            table: &table,
        };

        let compiled = metric
            .compile_over(self.metrics.people(), &Window::legacy(), over)
            .map_err(CustomError::Compile)?;

        let mut result = self
            .metrics
            .run(&compiled)
            .await
            .map_err(CustomError::Run)?;
        result.clock = EffectiveClock::of(metric, &declaration);

        Ok(result)
    }

    /// The declaration and the table of a dataset that is ready to be read.
    ///
    /// A dataset that is absent, still being made or being removed answers
    /// nothing: a run over it would read a table that is not there yet or is
    /// about to go.
    async fn ready_dataset(&self, named: &str) -> Result<(Declaration, String), CustomError> {
        let ready = datasets::ready(self.datasets, named)
            .await
            .ok_or_else(|| CustomError::DatasetNotReady(named.to_owned()))?;

        Ok((ready.declaration, ready.table))
    }

    async fn undated_over(
        &self,
        metric: &MetricQuery,
        over: Over<'_>,
    ) -> Result<UndatedCount, CustomError> {
        let Some(query) = metric
            .undated_query(TableEngine::Other, Some(over))
            .map_err(CustomError::Compile)?
        else {
            return Ok(UndatedCount::default());
        };

        self.metrics.undated(&query).await.map_err(CustomError::Run)
    }

    /// Which engine holds the metric's table, so a replacing one is read
    /// through `FINAL` rather than counted twice.
    async fn engine_of(&self, metric: &MetricQuery) -> Result<TableEngine, CustomError> {
        self.catalog
            .engine_of(&metric.qualified())
            .await
            .map_err(CustomError::Catalog)
    }

    async fn undated_of(
        &self,
        metric: &MetricQuery,
        request: &WindowRequest,
        engine: TableEngine,
    ) -> Result<UndatedCount, CustomError> {
        if !request.is_ranged() {
            return Ok(UndatedCount::default());
        }

        let Some(query) = metric
            .undated_query(engine, None)
            .map_err(CustomError::Compile)?
        else {
            return Ok(UndatedCount::default());
        };

        self.metrics.undated(&query).await.map_err(CustomError::Run)
    }
}
