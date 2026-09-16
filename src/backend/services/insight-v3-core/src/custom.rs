//! The metric, widget and dashboard operations, over the stores they need.

use chrono::Utc;
use serde_json::Value;
use thiserror::Error;

use crate::catalog::{Catalog, CatalogError, TableEngine, TableSchema};
use crate::dashboard::Item;
use crate::definitions::{
    DefinitionKind, DefinitionName, DefinitionStoreError, Definitions, NamePage, Page,
};
use crate::metric_query::{
    MetricQuery, MetricQueryError, MetricRunError, MetricRunner, People, RunResult,
};
use crate::time_window::{RequestedRange, WindowError, WindowRequest};
use crate::undated::UndatedCount;
use crate::widget::{Widget, WidgetError};

#[cfg(test)]
mod tests;

#[derive(Debug, Error)]
pub(crate) enum CustomError {
    #[error("{} `{name}` was not found", kind.singular())]
    NotFound { kind: DefinitionKind, name: String },
    #[error("still in use by {}", used_by.join(", "))]
    InUse { used_by: Vec<String> },
    #[error(transparent)]
    Widget(WidgetError),
    #[error("definition body is not valid: {0}")]
    Body(serde_json::Error),
    #[error(transparent)]
    Compile(MetricQueryError),
    #[error(transparent)]
    Run(MetricRunError),
    #[error(transparent)]
    Store(DefinitionStoreError),
    #[error(transparent)]
    Catalog(CatalogError),
    #[error("dashboard time range: {0}")]
    Range(WindowError),
}

impl CustomError {
    /// Whether the caller can act on this, which decides whether it is worth
    /// logging: a refusal the caller caused is answered, not recorded.
    pub(crate) fn is_about_the_caller(&self) -> bool {
        match self {
            Self::NotFound { .. }
            | Self::InUse { .. }
            | Self::Widget(_)
            | Self::Body(_)
            | Self::Range(_)
            | Self::Compile(_) => true,
            Self::Run(_) | Self::Store(_) | Self::Catalog(_) => false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Surfaces<'a> {
    definitions: &'a dyn Definitions,
    metrics: &'a MetricRunner,
    catalog: &'a Catalog,
}

impl<'a> Surfaces<'a> {
    pub(crate) fn new(
        definitions: &'a dyn Definitions,
        metrics: &'a MetricRunner,
        catalog: &'a Catalog,
    ) -> Self {
        Self {
            definitions,
            metrics,
            catalog,
        }
    }

    /// One page of the names of this kind matching `needle`, or of all of
    /// them when it is blank — a search box that is empty is not a search for
    /// nothing.
    pub(crate) async fn page(
        &self,
        kind: DefinitionKind,
        needle: &str,
        page: Page,
    ) -> Result<NamePage, CustomError> {
        self.definitions
            .page(kind, needle.trim(), page)
            .await
            .map_err(CustomError::Store)
    }

    pub(crate) async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, CustomError> {
        self.definitions
            .list(kind)
            .await
            .map_err(CustomError::Store)
    }

    pub(crate) async fn get(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Value, CustomError> {
        self.definitions
            .get(kind, name)
            .await
            .map_err(CustomError::Store)?
            .ok_or_else(|| CustomError::NotFound {
                kind,
                name: name.as_str().to_owned(),
            })
    }

    /// Replaces what a dashboard draws, keeping its title.
    ///
    /// The order of the widgets, and the headings and prose between them, are
    /// one list - so laying a board out is sending that list, not editing a
    /// body around it.
    pub(crate) async fn arrange(
        &self,
        name: &DefinitionName,
        items: &[Item],
    ) -> Result<Value, CustomError> {
        let previous = self.get(DefinitionKind::Dashboard, name).await?;

        for drawn in items.iter().filter_map(Item::widget) {
            let widget = DefinitionName::parse(drawn).map_err(|_| CustomError::NotFound {
                kind: DefinitionKind::Widget,
                name: drawn.to_owned(),
            })?;
            if self
                .definitions
                .get(DefinitionKind::Widget, &widget)
                .await
                .map_err(CustomError::Store)?
                .is_none()
            {
                return Err(CustomError::NotFound {
                    kind: DefinitionKind::Widget,
                    name: drawn.to_owned(),
                });
            }
        }

        let body = crate::dashboard::laid_out(&previous, items).map_err(CustomError::Body)?;
        self.definitions
            .put(DefinitionKind::Dashboard, name, &body)
            .await
            .map_err(CustomError::Store)?;

        Ok(body)
    }

    pub(crate) async fn put(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
        body: &Value,
    ) -> Result<(), CustomError> {
        if kind == DefinitionKind::Widget {
            self.check_widget(body).await?;
        }
        if kind == DefinitionKind::Metric {
            check_metric(body, self.metrics.people())?;
        }
        if kind == DefinitionKind::Dashboard {
            check_dashboard(body)?;
        }

        self.definitions
            .put(kind, name, body)
            .await
            .map_err(CustomError::Store)
    }

    pub(crate) async fn delete(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<(), CustomError> {
        let used_by = self.dependents_of(kind, name).await?;
        if !used_by.is_empty() {
            return Err(CustomError::InUse { used_by });
        }

        let removed = self
            .definitions
            .delete(kind, name)
            .await
            .map_err(CustomError::Store)?;

        if removed {
            return Ok(());
        }

        Err(CustomError::NotFound {
            kind,
            name: name.as_str().to_owned(),
        })
    }

    /// Runs a stored metric over the window the caller asked for, resolved
    /// against the wall clock, so a named range means the period it is named
    /// after whether or not the data reaches that far.
    pub(crate) async fn run_metric(
        &self,
        name: &DefinitionName,
        request: &WindowRequest,
    ) -> Result<RunResult, CustomError> {
        let body = self.get(DefinitionKind::Metric, name).await?;

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

    pub(crate) async fn check_widget(&self, body: &Value) -> Result<(), CustomError> {
        let widget: Widget = serde_json::from_value(body.clone())
            .map_err(|error| CustomError::Widget(error.into()))?;

        let metric_name = DefinitionName::parse(widget.metric())
            .map_err(|_| CustomError::Widget(WidgetError::NoMetric(widget.metric().to_owned())))?;

        let stored = self
            .definitions
            .get(DefinitionKind::Metric, &metric_name)
            .await
            .map_err(CustomError::Store)?
            .ok_or_else(|| {
                CustomError::Widget(WidgetError::NoMetric(widget.metric().to_owned()))
            })?;

        let metric: MetricQuery =
            serde_json::from_value(stored).map_err(|error| CustomError::Widget(error.into()))?;

        widget.check_against(&metric).map_err(CustomError::Widget)
    }

    pub(crate) async fn dependents_of(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Vec<String>, CustomError> {
        let Some((holder, needle)) = held_by(kind) else {
            return Ok(Vec::new());
        };

        let mut used_by = Vec::new();
        for holder_name in self.list(holder).await? {
            let Ok(parsed) = DefinitionName::parse(&holder_name) else {
                continue;
            };
            let Some(body) = self
                .definitions
                .get(holder, &parsed)
                .await
                .map_err(CustomError::Store)?
            else {
                continue;
            };

            let names: Vec<String> = match holder {
                // A board names its widgets in an item list or in the older
                // shorthand, and either one is still drawing them.
                DefinitionKind::Dashboard => crate::dashboard::widgets(&body),
                _ => match body.get(needle) {
                    Some(Value::String(one)) => vec![one.clone()],
                    Some(Value::Array(many)) => many
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect(),
                    _ => Vec::new(),
                },
            };
            if names.iter().any(|held| held == name.as_str()) {
                used_by.push(holder_name);
            }
        }

        Ok(used_by)
    }
}

/// Everything a metric must satisfy to run, checked before it is stored: a
/// body kept here that cannot compile answers nothing whenever it is read.
fn check_metric(body: &Value, people: &People) -> Result<(), CustomError> {
    let metric = serde_json::from_value::<MetricQuery>(body.clone()).map_err(CustomError::Body)?;

    metric.check_window().map_err(CustomError::Compile)?;
    metric.compile(people).map_err(CustomError::Compile)?;

    Ok(())
}

/// What a stored dashboard says about time, checked before it is stored: a
/// board offering a range the server cannot resolve draws a picker whose
/// buttons refuse every widget behind them.
fn check_dashboard(body: &Value) -> Result<(), CustomError> {
    let offered = body
        .get("time_ranges")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let default = body.get("default_range");

    for token in offered.iter().chain(default) {
        let Some(token) = token.as_str() else {
            return Err(CustomError::Range(WindowError::Range(token.to_string())));
        };
        RequestedRange::parse(token).map_err(CustomError::Range)?;
    }

    Ok(())
}

/// Which kind names this one, and under which field.
///
/// A widget names its metric; a dashboard names its widgets. Nothing names a
/// dashboard.
pub(crate) fn held_by(kind: DefinitionKind) -> Option<(DefinitionKind, &'static str)> {
    match kind {
        DefinitionKind::Metric => Some((DefinitionKind::Widget, "metric")),
        DefinitionKind::Widget => Some((DefinitionKind::Dashboard, "widgets")),
        DefinitionKind::Dashboard => None,
    }
}
