//! Running a stored metric over a window the reader picked.

use chrono::Utc;

use super::datasets::{self, Datasets};
use super::definition::{DefinitionKind, DefinitionName, Lookup};
use super::kinds::dataset::declaration::Declaration;
use super::kinds::metric::answerable::{self, EffectiveClock};
use super::query::metric_query::over::Over;
use super::query::metric_query::{ColumnKind, CompiledQuery, MetricQuery, MetricRunner, RunResult};
use super::query::time_window::{Window, WindowRequest};
use super::query::undated::UndatedCount;
use super::surfaces::CustomError;
use crate::domain::query::metric_query::TableEngine;

/// A metric compiled for a reader that orders and pages it: the query with
/// nothing cut off it, what each column holds, and where the rows come from.
#[derive(Debug)]
pub(crate) struct Prepared {
    pub(crate) compiled: CompiledQuery,
    pub(crate) kinds: Vec<(String, ColumnKind)>,
    /// The order the metric asks for itself, if it asks for one.
    pub(crate) own_order: Option<(String, bool)>,
    pub(crate) source: Source,
    /// The body as stored, which is what a page has to be bound to.
    pub(crate) body: serde_json::Value,
}

/// Where a metric's rows come from, as far as a reader paging through them
/// needs to know.
#[derive(Debug)]
pub(crate) enum Source {
    Table {
        database: Option<String>,
        table: String,
    },
    Dataset {
        /// The table the records are in, named for the attempt that made it.
        table: String,
    },
}

/// Everything a metric needs to answer: the definition it is stored as, the
/// dataset it reads, and the warehouse the records are kept in.
#[derive(Debug)]
pub(crate) struct MetricRuns<'a> {
    definitions: &'a dyn Lookup,
    metrics: &'a MetricRunner,
    datasets: &'a dyn Datasets,
    /// The database every dataset's records are kept in.
    datasets_database: &'a str,
}

impl<'a> MetricRuns<'a> {
    pub(crate) fn new(
        definitions: &'a dyn Lookup,
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

    /// The window as of now: a named range means the period it is named
    /// after whether or not the data reaches that far.
    pub(crate) fn resolve(request: &WindowRequest) -> Result<Window, CustomError> {
        request
            .resolve(Utc::now())
            .map_err(|error| CustomError::Compile(error.into()))
    }

    /// A dataset's records as a metric is compiled against them.
    fn over_of<'d>(&'d self, declaration: &'d Declaration, table: &'d str) -> Over<'d> {
        Over {
            declaration,
            database: self.datasets_database,
            table,
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

        let Some(named) = metric.dataset() else {
            return self.run_over_table(&metric, request).await;
        };

        self.run_over_dataset(&metric, named, request).await
    }

    /// Compiles a stored metric for a reader that orders and pages it, over
    /// a window already resolved - the same one for every page of a walk.
    /// Nothing is run.
    pub(crate) async fn prepare(
        &self,
        name: &DefinitionName,
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

    /// Runs a metric over the warehouse table it names, as the table is.
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

    /// The rows of a compiled run, and beside them, for a ranged one, how many
    /// rows the window left out. The two reads are independent.
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
        let over = self.over_of(&declaration, &table);

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
        result.clock = EffectiveClock::of(metric, &declaration);

        Ok(result)
    }

    /// Runs a metric nobody stored: the assistant's own, written to answer one
    /// question and kept nowhere.
    pub(crate) async fn answer(&self, metric: &MetricQuery) -> Result<RunResult, CustomError> {
        let Some(named) = metric.dataset() else {
            let unranged = WindowRequest::parse(None, None)
                .map_err(|error| CustomError::Compile(error.into()))?;

            return self.run_over_table(metric, &unranged).await;
        };
        let (declaration, table) = self.ready_dataset(named).await?;

        // Nobody stored this one, so nothing has checked it against the
        // dataset yet. A query the declaration cannot answer is refused here
        // rather than sent to the warehouse to fail there.
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

    /// The declaration and the table of a dataset that is ready to be read.
    ///
    /// A dataset that is absent, still being made or being removed answers
    /// nothing: a run over it would read a table that is not there yet or is
    /// about to go.
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
