//! Storing, reading and removing definitions, whatever kind they are.

use serde_json::Value;
use thiserror::Error;

use crate::domain::datasets::{self, Datasets};
use crate::domain::definition::arriving::Arriving;
use crate::domain::definition::{
    Definition, DefinitionId, DefinitionKind, DefinitionLabel, DefinitionStoreError, Definitions,
    ListedPage, Page, StoredDefinition,
};
use crate::domain::kinds::dashboard::Item;
use crate::domain::kinds::metric::CompileAgainst;
use crate::domain::kinds::metric::answerable::EffectiveClock;
use crate::domain::kinds::widget::WidgetError;
use crate::domain::kinds::{self, KindError};
use crate::domain::query::metric_query::{MetricQuery, MetricQueryError, MetricRunError, People};
use crate::domain::query::time_window::WindowError;
use crate::domain::violation::Violation;

mod duplicate;
#[cfg(test)]
mod tests;

#[derive(Debug, Error)]
pub(crate) enum CustomError {
    #[error("{} `{id}` was not found", kind.singular())]
    NotFound { kind: DefinitionKind, id: String },
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
    #[error("dashboard time range: {0}")]
    Range(WindowError),
    #[error("no dataset named `{0}` is ready to be read")]
    DatasetNotReady(String),
    #[error(transparent)]
    Datasets(crate::domain::datasets::DatasetStoreError),
    #[error("this dataset cannot answer that metric - {}", crate::domain::violation::said(.0))]
    Unanswerable(Vec<Violation>),
}

impl From<KindError> for CustomError {
    fn from(error: KindError) -> Self {
        match error {
            KindError::Widget(source) => Self::Widget(source),
            KindError::Body(source) => Self::Body(source),
            KindError::Compile(source) => Self::Compile(source),
            KindError::Range(source) => Self::Range(source),
            KindError::Store(source) => Self::Store(source),
            KindError::DatasetNotReady(named) => Self::DatasetNotReady(named),
            KindError::Datasets(source) => Self::Datasets(source),
            KindError::Unanswerable(violations) => Self::Unanswerable(violations),
        }
    }
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
            | Self::DatasetNotReady(_)
            | Self::Unanswerable(_)
            | Self::Compile(_) => true,
            Self::Run(_) | Self::Store(_) | Self::Datasets(_) => false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Surfaces<'a> {
    definitions: &'a dyn Definitions,
    datasets: &'a dyn Datasets,
    /// What a metric is compiled against before it is stored.
    over: CompileAgainst<'a>,
}

impl<'a> Surfaces<'a> {
    pub(crate) fn new(
        definitions: &'a dyn Definitions,
        datasets: &'a dyn Datasets,
        datasets_database: &'a str,
        people: &'a People,
    ) -> Self {
        Self {
            definitions,
            datasets,
            over: CompileAgainst {
                database: datasets_database,
                people,
            },
        }
    }

    /// One page of the definitions of this kind matching `needle`, or of all
    /// of them when it is blank — a search box that is empty is not a search
    /// for nothing.
    pub(crate) async fn page(
        &self,
        kind: DefinitionKind,
        needle: &str,
        page: Page,
    ) -> Result<ListedPage, CustomError> {
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
        id: &DefinitionId,
    ) -> Result<Value, CustomError> {
        self.definitions
            .get(kind, id)
            .await
            .map_err(CustomError::Store)?
            .ok_or_else(|| not_found(kind, id))
    }

    pub(crate) async fn read(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<StoredDefinition, CustomError> {
        self.definitions
            .read(kind, id)
            .await
            .map_err(CustomError::Store)?
            .ok_or_else(|| not_found(kind, id))
    }

    /// Replaces what a dashboard draws, keeping its title and its name.
    ///
    /// The order of the widgets, and the headings and prose between them, are
    /// one list - so laying a board out is sending that list, not editing a
    /// body around it.
    pub(crate) async fn arrange(
        &self,
        id: &DefinitionId,
        items: &[Item],
    ) -> Result<Value, CustomError> {
        let previous = self.read(DefinitionKind::Dashboard, id).await?;

        for drawn in items.iter().filter_map(Item::widget) {
            let missing = || CustomError::NotFound {
                kind: DefinitionKind::Widget,
                id: drawn.to_owned(),
            };
            let widget = DefinitionId::parse(drawn).map_err(|_| missing())?;
            if self
                .definitions
                .get(DefinitionKind::Widget, &widget)
                .await
                .map_err(CustomError::Store)?
                .is_none()
            {
                return Err(missing());
            }
        }

        let body = kinds::dashboard::laid_out(&previous.body, items).map_err(CustomError::Body)?;
        let label =
            DefinitionLabel::parse(&previous.name).unwrap_or_else(|_| DefinitionLabel::of_id(id));
        self.store_replacing(DefinitionKind::Dashboard, id, &label, &body)
            .await?;

        Ok(body)
    }

    /// Stores a new definition under a fresh id, after checking its body.
    pub(crate) async fn create(
        &self,
        kind: DefinitionKind,
        name: &DefinitionLabel,
        body: &Value,
    ) -> Result<DefinitionId, CustomError> {
        kinds::check(kind, body, self.definitions, self.datasets, self.over).await?;

        let id = DefinitionId::generate();
        self.definitions
            .create(kind, &id, name, body)
            .await
            .map_err(CustomError::Store)?;

        Ok(id)
    }

    /// Replaces what an existing definition is called and holds, after
    /// checking its body.
    pub(crate) async fn replace(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &Value,
    ) -> Result<(), CustomError> {
        kinds::check(kind, body, self.definitions, self.datasets, self.over).await?;

        self.store_replacing(kind, id, name, body).await
    }

    async fn store_replacing(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &Value,
    ) -> Result<(), CustomError> {
        let replaced = self
            .definitions
            .replace(kind, id, name, body)
            .await
            .map_err(CustomError::Store)?;

        if replaced {
            return Ok(());
        }

        Err(not_found(kind, id))
    }

    /// The clock a window over this definition selects by, when it has one.
    ///
    /// Only a metric does, and a metric over a dataset may inherit it, so its
    /// stored body is not enough to tell a reader whether a card drawn from it
    /// follows the board's window.
    pub(crate) async fn clock_of(
        &self,
        kind: DefinitionKind,
        body: &Value,
    ) -> Option<EffectiveClock> {
        match kind {
            DefinitionKind::Metric => {}
            DefinitionKind::Widget | DefinitionKind::Dashboard => return None,
        }

        let metric: MetricQuery = serde_json::from_value(body.clone()).ok()?;
        let Some(named) = metric.dataset() else {
            return EffectiveClock::of_table(&metric);
        };
        let held = datasets::ready(self.datasets, named).await;
        let ready = match held {
            Ok(ready) => ready?,
            Err(error) => {
                tracing::error!(error = ?error, "could not read a metric's dataset");
                return None;
            }
        };

        EffectiveClock::of(&metric, &ready.declaration)
    }

    pub(crate) async fn delete(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<(), CustomError> {
        let used_by = self.dependents_of(kind, id).await?;
        if !used_by.is_empty() {
            return Err(CustomError::InUse {
                used_by: used_by.into_iter().map(|holder| holder.name).collect(),
            });
        }

        let removed = self
            .definitions
            .delete(kind, id)
            .await
            .map_err(CustomError::Store)?;

        if removed {
            return Ok(());
        }

        Err(not_found(kind, id))
    }

    /// Whether every body in a batch can be stored, before any of it is.
    ///
    /// Each is checked against the store as the batch will leave it, so a
    /// widget may draw a metric the same batch writes — and so one refusal
    /// stores none of it.
    pub(crate) async fn check_batch(&self, batch: &[Definition]) -> Result<(), CustomError> {
        let arriving = Arriving::new(self.definitions, batch);

        for arriving_definition in batch {
            kinds::check(
                arriving_definition.kind,
                &arriving_definition.body,
                &arriving,
                self.datasets,
                self.over,
            )
            .await?;
        }

        Ok(())
    }

    /// Every stored definition that names this one: what a removal would
    /// break.
    pub(crate) async fn dependents_of(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<Vec<Holder>, CustomError> {
        let mut holders = Vec::new();

        for holder in kinds::referred_to_by(kind) {
            holders.extend(self.holders_among(*holder, kind, id).await?);
        }

        Ok(holders)
    }

    /// The definitions of one kind that name `id`.
    async fn holders_among(
        &self,
        holder: DefinitionKind,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<Vec<Holder>, CustomError> {
        let mut found = Vec::new();

        for holder_id in self.list(holder).await? {
            let Ok(parsed) = DefinitionId::parse(&holder_id) else {
                continue;
            };
            let Some(stored) = self
                .definitions
                .read(holder, &parsed)
                .await
                .map_err(CustomError::Store)?
            else {
                continue;
            };

            if names(holder, &stored.body, kind, id) {
                found.push(Holder {
                    kind: holder,
                    id: holder_id,
                    name: stored.name,
                });
            }
        }

        Ok(found)
    }
}

/// A stored definition that names another.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Holder {
    pub(crate) kind: DefinitionKind,
    pub(crate) id: String,
    pub(crate) name: String,
}

fn not_found(kind: DefinitionKind, id: &DefinitionId) -> CustomError {
    CustomError::NotFound {
        kind,
        id: id.as_str().to_owned(),
    }
}

/// Whether this body names that definition.
fn names(holder: DefinitionKind, body: &Value, kind: DefinitionKind, id: &DefinitionId) -> bool {
    kinds::refers_to(holder, body)
        .into_iter()
        .any(|reference| reference.kind == kind && reference.id == id.as_str())
}
