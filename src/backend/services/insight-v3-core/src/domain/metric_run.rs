//! Running a stored metric over a window the reader picked.

use chrono::Utc;

use super::definition::{DefinitionKind, DefinitionName, Lookup};
use super::query::metric_query::{MetricQuery, MetricQueryError, MetricRunner, RunResult};
use super::query::time_window::WindowRequest;
use super::query::undated::UndatedCount;
use super::surfaces::CustomError;
use crate::store::catalog::{Catalog, TableEngine, TableSchema};

/// Everything a metric needs to answer: the definition it is stored as, the
/// warehouse that holds the table, and the catalogue that says how.
#[derive(Debug)]
pub(crate) struct MetricRuns<'a> {
    definitions: &'a dyn Lookup,
    metrics: &'a MetricRunner,
    catalog: &'a Catalog,
}

impl<'a> MetricRuns<'a> {
    pub(crate) fn new(
        definitions: &'a dyn Lookup,
        metrics: &'a MetricRunner,
        catalog: &'a Catalog,
    ) -> Self {
        Self {
            definitions,
            metrics,
            catalog,
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
            .compile_window(self.metrics.people(), &window, engine)
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

        let Some(query) = metric.undated_query(engine).map_err(CustomError::Compile)? else {
            return Ok(UndatedCount::default());
        };

        self.metrics.undated(&query).await.map_err(CustomError::Run)
    }

    pub(crate) async fn tables(&self) -> Result<Vec<TableSchema>, CustomError> {
        self.catalog.tables().await.map_err(CustomError::Catalog)
    }
}
