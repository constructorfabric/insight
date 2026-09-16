//! The metric, widget and dashboard operations, over the stores they need.

use chrono::Utc;
use serde_json::Value;
use thiserror::Error;

use crate::definitions::arriving::Arriving;
use crate::definitions::{
    DefinitionKind, DefinitionName, DefinitionStoreError, Definitions, NamePage, Page,
};
use crate::domain::kinds::dashboard::Item;
use crate::domain::kinds::widget::WidgetError;
use crate::domain::kinds::{self, KindError, Reference};
use crate::domain::query::metric_query::{
    MetricQuery, MetricQueryError, MetricRunError, MetricRunner, RunResult,
};
use crate::domain::query::time_window::{WindowError, WindowRequest};
use crate::domain::query::undated::UndatedCount;
use crate::store::catalog::{Catalog, CatalogError, TableEngine, TableSchema};

#[cfg(test)]
mod tests;

impl From<KindError> for CustomError {
    fn from(error: KindError) -> Self {
        match error {
            KindError::Widget(source) => Self::Widget(source),
            KindError::Body(source) => Self::Body(source),
            KindError::Compile(source) => Self::Compile(source),
            KindError::Range(source) => Self::Range(source),
            KindError::Store(source) => Self::Store(source),
        }
    }
}

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

        let body = kinds::dashboard::laid_out(&previous, items).map_err(CustomError::Body)?;
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
        kinds::check(kind, body, self.definitions).await?;

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
            return Err(CustomError::InUse {
                used_by: used_by.into_iter().map(|holder| holder.name).collect(),
            });
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

    /// Whether every body in a batch can be stored, before any of it is.
    ///
    /// Each is checked against the store as the batch will leave it, so a
    /// widget may draw a metric the same batch writes — and so one refusal
    /// stores none of it.
    pub(crate) async fn check_batch(
        &self,
        batch: &[(DefinitionKind, DefinitionName, Value)],
    ) -> Result<(), CustomError> {
        let arriving = Arriving::new(self.definitions, batch);

        for (kind, _, body) in batch {
            kinds::check(*kind, body, &arriving).await?;
        }

        Ok(())
    }

    pub(crate) async fn dependents_of(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Vec<Reference>, CustomError> {
        let mut used_by = Vec::new();

        for holder in kinds::referred_to_by(kind) {
            for holder_name in self.list(*holder).await? {
                let Ok(parsed) = DefinitionName::parse(&holder_name) else {
                    continue;
                };
                let Some(body) = self
                    .definitions
                    .get(*holder, &parsed)
                    .await
                    .map_err(CustomError::Store)?
                else {
                    continue;
                };

                let names_it = kinds::refers_to(*holder, &body)
                    .into_iter()
                    .any(|reference| reference.kind == kind && reference.name == name.as_str());

                if names_it {
                    used_by.push(Reference::new(*holder, holder_name));
                }
            }
        }

        Ok(used_by)
    }
}
