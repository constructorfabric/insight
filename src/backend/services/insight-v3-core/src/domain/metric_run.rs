//! Running a stored metric over a window the reader picked.

use chrono::Utc;

use super::datasets::{self, Datasets};
use super::definition::{DefinitionId, DefinitionKind, Definitions, StoredDefinition};
use super::kinds::dataset::declaration::Declaration;
use super::kinds::metric::answerable::{self, EffectiveClock};
use super::kinds::metric::composite::{self, Children};
use super::query::composite::{Composite, CompositeError};
use super::query::metric::Metric;
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

/// What a stored metric's rows hold, read from its definition alone: each
/// result column and its kind, and the columns the rows are grouped by.
#[derive(Debug)]
pub(crate) struct Columns {
    pub(crate) kinds: Vec<(String, ColumnKind)>,
    pub(crate) groups: Vec<String>,
    /// Where a run of it stops without the metric having asked to.
    pub(crate) imposed_limit: Option<usize>,
}

/// A stored metric's rows, the columns of the definition they came from,
/// and what that definition is called.
#[derive(Debug)]
pub(crate) struct NamedRun {
    pub(crate) result: RunResult,
    pub(crate) columns: Columns,
    pub(crate) name: String,
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
    definitions: &'a dyn Definitions,
    metrics: &'a MetricRunner,
    datasets: &'a dyn Datasets,
    /// The database every dataset's records are kept in.
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
    async fn body_of(&self, id: &DefinitionId) -> Result<serde_json::Value, CustomError> {
        self.definitions
            .get(DefinitionKind::Metric, id)
            .await
            .map_err(CustomError::Store)?
            .ok_or_else(|| missing_metric(id))
    }

    /// A stored metric, read as a plain query or a composite.
    async fn metric_of(&self, id: &DefinitionId) -> Result<Metric, CustomError> {
        Metric::parse(self.body_of(id).await?).map_err(CustomError::Body)
    }

    /// Every metric a composite combines, read and admitted now.
    async fn children_of(&self, composite: &Composite) -> Result<Children, CustomError> {
        composite::children(composite, self.definitions, self.datasets)
            .await
            .map_err(CustomError::from)
    }

    /// The stored metric with what it is called, or the refusal that there
    /// is none.
    async fn stored_of(&self, id: &DefinitionId) -> Result<StoredDefinition, CustomError> {
        self.definitions
            .read(DefinitionKind::Metric, id)
            .await
            .map_err(CustomError::Store)?
            .ok_or_else(|| missing_metric(id))
    }

    /// The columns a stored metric answers, from its definition and its
    /// dataset's declaration, or from the metrics a composite combines.
    /// Nothing is run.
    pub(crate) async fn columns(&self, name: &DefinitionId) -> Result<Columns, CustomError> {
        let metric = self.metric_of(name).await?;

        let kinds = match &metric {
            Metric::Query(query) => self.query_kinds(query).await?,
            Metric::Composite(composite) => self.children_of(composite).await?.column_kinds(false),
        };

        Ok(columns_of(&metric, kinds))
    }

    /// What a plain metric's columns hold, from its dataset's declaration
    /// when it reads one.
    async fn query_kinds(
        &self,
        query: &MetricQuery,
    ) -> Result<Vec<(String, ColumnKind)>, CustomError> {
        let Some(named) = query.dataset() else {
            query.check_table().map_err(CustomError::Compile)?;
            return Ok(query.column_kinds(None, false));
        };
        let (declaration, _) = self.ready_dataset(named).await?;

        Ok(query.column_kinds(Some(&declaration), false))
    }

    /// Runs a stored metric as [`MetricRuns::run`] does, and answers the
    /// columns and the name of the same definition the rows came from.
    pub(crate) async fn run_named(
        &self,
        id: &DefinitionId,
        request: &WindowRequest,
    ) -> Result<NamedRun, CustomError> {
        let stored = self.stored_of(id).await?;
        let metric = Metric::parse(stored.body).map_err(CustomError::Body)?;

        let (result, kinds) = match &metric {
            Metric::Query(query) => self.run_query_named(query, request).await?,
            Metric::Composite(composite) => {
                let children = self.children_of(composite).await?;
                let kinds = children.column_kinds(false);
                (
                    self.run_composite(composite, children, request).await?,
                    kinds,
                )
            }
        };

        Ok(NamedRun {
            result,
            columns: columns_of(&metric, kinds),
            name: stored.name,
        })
    }

    /// A plain metric's rows, and what each of its columns holds.
    async fn run_query_named(
        &self,
        query: &MetricQuery,
        request: &WindowRequest,
    ) -> Result<(RunResult, Vec<(String, ColumnKind)>), CustomError> {
        let Some(named) = query.dataset() else {
            let result = self.run_over_table(query, request).await?;
            return Ok((result, query.column_kinds(None, false)));
        };
        let (declaration, table) = self.ready_dataset(named).await?;
        let result = self
            .run_over_ready_dataset(query, &declaration, &table, request)
            .await?;

        Ok((result, query.column_kinds(Some(&declaration), false)))
    }

    /// Runs a stored metric over the window the caller asked for, resolved
    /// against the wall clock, so a named range means the period it is named
    /// after whether or not the data reaches that far.
    pub(crate) async fn run(
        &self,
        name: &DefinitionId,
        request: &WindowRequest,
    ) -> Result<RunResult, CustomError> {
        let metric = match self.metric_of(name).await? {
            Metric::Query(query) => query,
            Metric::Composite(composite) => {
                let children = self.children_of(&composite).await?;
                return self.run_composite(&composite, children, request).await;
            }
        };

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
        name: &DefinitionId,
        window: &Window,
    ) -> Result<Prepared, CustomError> {
        let body = self.body_of(name).await?;
        let metric = match Metric::parse(body.clone()).map_err(CustomError::Body)? {
            Metric::Query(query) => query,
            Metric::Composite(_) => return Err(CustomError::Composite(CompositeError::NoRecords)),
        };
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

    /// Runs a composite once: each child compiled over the window as a
    /// subquery, stacked and folded. It counts no undated rows of its own.
    async fn run_composite(
        &self,
        composite: &Composite,
        mut children: Children,
        request: &WindowRequest,
    ) -> Result<RunResult, CustomError> {
        let window = Self::resolve(request)?;
        children
            .read_engines(self.metrics)
            .await
            .map_err(CustomError::Run)?;
        let compiled = children
            .compile(
                composite,
                self.metrics.people(),
                &window,
                self.datasets_database,
            )
            .map_err(CustomError::Composite)?;

        let mut result = self
            .metrics
            .run(&compiled)
            .await
            .map_err(CustomError::Run)?;
        result.clock = children.clock();

        Ok(result)
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

fn missing_metric(id: &DefinitionId) -> CustomError {
    CustomError::NotFound {
        kind: DefinitionKind::Metric,
        id: id.as_str().to_owned(),
    }
}

/// What a definition says its rows hold, given what each column holds as
/// what it reads decides.
fn columns_of(metric: &Metric, kinds: Vec<(String, ColumnKind)>) -> Columns {
    Columns {
        kinds,
        groups: metric.groups().to_vec(),
        imposed_limit: metric
            .imposed_limit()
            .and_then(|limit| usize::try_from(limit).ok()),
    }
}
