use chrono::Utc;

use super::datasets::{self, Datasets};
use super::definition::{DefinitionId, DefinitionKind, Definitions, StoredDefinition};
use super::kinds::dataset::declaration::Declaration;
use super::kinds::metric::answerable::{self, EffectiveClock};
use super::query::metric_query::over::Over;
use super::query::metric_query::{ColumnKind, CompiledQuery, MetricQuery, MetricRunner, RunResult};
use super::query::time_window::{Window, WindowRequest};
use super::query::undated::UndatedCount;
use super::surfaces::CustomError;
use crate::domain::query::metric_query::TableEngine;

#[derive(Debug)]
pub(crate) struct Prepared {
    pub(crate) compiled: CompiledQuery,
    pub(crate) kinds: Vec<(String, ColumnKind)>,
    pub(crate) own_order: Option<(String, bool)>,
    pub(crate) source: Source,
    pub(crate) body: serde_json::Value,
}

#[derive(Debug)]
pub(crate) struct Columns {
    pub(crate) kinds: Vec<(String, ColumnKind)>,
    pub(crate) groups: Vec<String>,
    pub(crate) imposed_limit: Option<usize>,
}

#[derive(Debug)]
pub(crate) struct NamedRun {
    pub(crate) result: RunResult,
    pub(crate) columns: Columns,
    pub(crate) name: String,
}

#[derive(Debug)]
pub(crate) enum Source {
    Table {
        database: Option<String>,
        table: String,
    },
    Dataset {
        table: String,
    },
}

#[derive(Debug)]
pub(crate) struct MetricRuns<'a> {
    definitions: &'a dyn Definitions,
    metrics: &'a MetricRunner,
    datasets: &'a dyn Datasets,
    datasets_database: &'a str,
}

impl<'a> MetricRuns<'a> {
    pub(crate) fn new(
        definitions: &'a dyn Definitions,
        metrics: &'a MetricRunner,
        datasets: &'a dyn Datasets,
        datasets_database: &'a str,
    ) -> Self {
        Self {
            definitions,
            metrics,
            datasets,
            datasets_database,
        }
    }

    pub(crate) fn resolve(request: &WindowRequest) -> Result<Window, CustomError> {
        request
            .resolve(Utc::now())
            .map_err(|error| CustomError::Compile(error.into()))
    }

    fn over_of<'d>(&'d self, declaration: &'d Declaration, table: &'d str) -> Over<'d> {
        Over {
            declaration,
            database: self.datasets_database,
            table,
        }
    }

    async fn body_of(&self, id: &DefinitionId) -> Result<serde_json::Value, CustomError> {
        self.definitions
            .get(DefinitionKind::Metric, id)
            .await
            .map_err(CustomError::Store)?
            .ok_or_else(|| missing_metric(id))
    }

    async fn stored_of(&self, id: &DefinitionId) -> Result<StoredDefinition, CustomError> {
        self.definitions
            .read(DefinitionKind::Metric, id)
            .await
            .map_err(CustomError::Store)?
            .ok_or_else(|| missing_metric(id))
    }

    pub(crate) async fn columns(&self, name: &DefinitionId) -> Result<Columns, CustomError> {
        let body = self.body_of(name).await?;
        let metric: MetricQuery = serde_json::from_value(body).map_err(CustomError::Body)?;

        let Some(named) = metric.dataset() else {
            metric.check_table().map_err(CustomError::Compile)?;
            return Ok(columns_of(&metric, None));
        };
        let (declaration, _) = self.ready_dataset(named).await?;

        Ok(columns_of(&metric, Some(&declaration)))
    }

    pub(crate) async fn run_named(
        &self,
        id: &DefinitionId,
        request: &WindowRequest,
    ) -> Result<NamedRun, CustomError> {
        let stored = self.stored_of(id).await?;
        let metric: MetricQuery = serde_json::from_value(stored.body).map_err(CustomError::Body)?;

        let Some(named) = metric.dataset() else {
            let result = self.run_over_table(&metric, request).await?;
            return Ok(NamedRun {
                result,
                columns: columns_of(&metric, None),
                name: stored.name,
            });
        };
        let (declaration, table) = self.ready_dataset(named).await?;
        let result = self
            .run_over_ready_dataset(&metric, &declaration, &table, request)
            .await?;

        Ok(NamedRun {
            result,
            columns: columns_of(&metric, Some(&declaration)),
            name: stored.name,
        })
    }

    pub(crate) async fn run(
        &self,
        name: &DefinitionId,
        request: &WindowRequest,
    ) -> Result<RunResult, CustomError> {
        let body = self.body_of(name).await?;

        let metric: MetricQuery = serde_json::from_value(body).map_err(CustomError::Body)?;

        let Some(named) = metric.dataset() else {
            return self.run_over_table(&metric, request).await;
        };

        self.run_over_dataset(&metric, named, request).await
    }

    pub(crate) async fn prepare(
        &self,
        name: &DefinitionId,
        window: &Window,
    ) -> Result<Prepared, CustomError> {
        let body = self.body_of(name).await?;
        let metric: MetricQuery =
            serde_json::from_value(body.clone()).map_err(CustomError::Body)?;
        let own_order = metric
            .own_order()
            .map(|(field, descending)| (field.to_owned(), descending));

        let Some(named) = metric.dataset() else {
            metric.check_table().map_err(CustomError::Compile)?;
            let engine = self
                .metrics
                .engine_of(metric.database(), metric.table_name())
                .await
                .map_err(CustomError::Run)?;
            let compiled = metric
                .compile_paged(self.metrics.people(), window, engine, None)
                .map_err(CustomError::Compile)?;
            let bucketed = window.grain().is_some() && metric.has_clock().unwrap_or(false);

            return Ok(Prepared {
                kinds: metric.column_kinds(None, bucketed),
                compiled,
                own_order,
                source: Source::Table {
                    database: metric.database().map(str::to_owned),
                    table: metric.table_name().to_owned(),
                },
                body,
            });
        };

        let (declaration, table) = self.ready_dataset(named).await?;
        let over = self.over_of(&declaration, &table);
        let compiled = metric
            .compile_paged(
                self.metrics.people(),
                window,
                TableEngine::Other,
                Some(over),
            )
            .map_err(CustomError::Compile)?;
        let bucketed = window.grain().is_some()
            && answerable::effective_clock(&metric, &declaration).is_some();

        Ok(Prepared {
            kinds: metric.column_kinds(Some(&declaration), bucketed),
            compiled,
            own_order,
            source: Source::Dataset { table },
            body,
        })
    }

    async fn run_over_table(
        &self,
        metric: &MetricQuery,
        request: &WindowRequest,
    ) -> Result<RunResult, CustomError> {
        metric.check_table().map_err(CustomError::Compile)?;

        let engine = self
            .metrics
            .engine_of(metric.database(), metric.table_name())
            .await
            .map_err(CustomError::Run)?;
        let window = Self::resolve(request)?;
        let compiled = metric
            .compile_window(self.metrics.people(), &window, engine, None)
            .map_err(CustomError::Compile)?;

        let mut result = self
            .counted(metric, &compiled, request.is_ranged(), engine, None)
            .await?;
        result.clock = EffectiveClock::of_table(metric);

        Ok(result)
    }

    async fn counted(
        &self,
        metric: &MetricQuery,
        compiled: &CompiledQuery,
        ranged: bool,
        engine: TableEngine,
        over: Option<Over<'_>>,
    ) -> Result<RunResult, CustomError> {
        let rows = async { self.metrics.run(compiled).await.map_err(CustomError::Run) };
        if !ranged {
            return rows.await;
        }

        let (mut result, undated) = tokio::try_join!(rows, self.undated_of(metric, engine, over))?;
        result.undated = Some(undated.count());

        Ok(result)
    }

    async fn run_over_dataset(
        &self,
        metric: &MetricQuery,
        named: &str,
        request: &WindowRequest,
    ) -> Result<RunResult, CustomError> {
        let (declaration, table) = self.ready_dataset(named).await?;

        self.run_over_ready_dataset(metric, &declaration, &table, request)
            .await
    }

    async fn run_over_ready_dataset(
        &self,
        metric: &MetricQuery,
        declaration: &Declaration,
        table: &str,
        request: &WindowRequest,
    ) -> Result<RunResult, CustomError> {
        let over = self.over_of(declaration, table);

        let window = Self::resolve(request)?;
        let compiled = metric
            .compile_over(self.metrics.people(), &window, over)
            .map_err(CustomError::Compile)?;

        let mut result = self
            .counted(
                metric,
                &compiled,
                request.is_ranged(),
                TableEngine::Other,
                Some(over),
            )
            .await?;
        result.clock = EffectiveClock::of(metric, declaration);

        Ok(result)
    }

    pub(crate) async fn answer(&self, metric: &MetricQuery) -> Result<RunResult, CustomError> {
        let Some(named) = metric.dataset() else {
            let unranged = WindowRequest::parse(None, None)
                .map_err(|error| CustomError::Compile(error.into()))?;

            return self.run_over_table(metric, &unranged).await;
        };
        let (declaration, table) = self.ready_dataset(named).await?;

        let violations = answerable::check(metric, &declaration);
        if !violations.is_empty() {
            return Err(CustomError::Unanswerable(violations));
        }

        let over = self.over_of(&declaration, &table);

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

    async fn ready_dataset(&self, named: &str) -> Result<(Declaration, String), CustomError> {
        let ready = datasets::ready(self.datasets, named)
            .await
            .map_err(CustomError::Datasets)?
            .ok_or_else(|| CustomError::DatasetNotReady(named.to_owned()))?;

        Ok((ready.declaration, ready.table))
    }

    async fn undated_of(
        &self,
        metric: &MetricQuery,
        engine: TableEngine,
        over: Option<Over<'_>>,
    ) -> Result<UndatedCount, CustomError> {
        let Some(query) = metric
            .undated_query(engine, over)
            .map_err(CustomError::Compile)?
        else {
            return Ok(UndatedCount::default());
        };

        self.metrics.undated(&query).await.map_err(CustomError::Run)
    }
}

fn missing_metric(id: &DefinitionId) -> CustomError {
    CustomError::NotFound {
        kind: DefinitionKind::Metric,
        id: id.as_str().to_owned(),
    }
}

fn columns_of(metric: &MetricQuery, declaration: Option<&Declaration>) -> Columns {
    Columns {
        kinds: metric.column_kinds(declaration, false),
        groups: metric.groups().to_vec(),
        imposed_limit: metric
            .imposed_limit()
            .and_then(|limit| usize::try_from(limit).ok()),
    }
}
