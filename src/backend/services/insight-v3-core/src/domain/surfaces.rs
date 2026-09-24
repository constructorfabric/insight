//! Storing, reading and renaming definitions, whatever kind they are.

use serde_json::Value;
use thiserror::Error;

use crate::domain::datasets::{self, Datasets};
use crate::domain::definition::arriving::Arriving;
use crate::domain::definition::{
    Change, Definition, DefinitionKind, DefinitionName, DefinitionStoreError, Definitions,
    NamePage, Page,
};
use crate::domain::kinds::dashboard::Item;
use crate::domain::kinds::metric::CompileAgainst;
use crate::domain::kinds::metric::answerable::EffectiveClock;
use crate::domain::kinds::widget::WidgetError;
use crate::domain::kinds::{self, KindError, Reference};
use crate::domain::query::metric_query::{MetricQuery, MetricQueryError, MetricRunError, People};
use crate::domain::query::time_window::WindowError;
use crate::domain::violation::Violation;

#[cfg(test)]
mod tests;

/// One definition that names another, and why it no longer works if it does
/// not.
#[derive(Debug)]
pub(crate) struct Dependent {
    pub(crate) reference: Reference,
    pub(crate) broken: Option<String>,
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
        kinds::check(kind, body, self.definitions, self.datasets, self.over).await?;

        self.definitions
            .put(kind, name, body)
            .await
            .map_err(CustomError::Store)
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

    /// Every definition that names this one: what a removal would break, and
    /// what a rename would rewrite.
    /// What still holds this definition, and whether each holder would be
    /// accepted as it stands.
    ///
    /// A widget names the columns its metric produces, so a metric that stops
    /// producing one leaves the widget drawing nothing. That write is not
    /// refused - a column could then never be renamed at all, since no widget
    /// may name a column its metric does not yet produce - so the damage is
    /// reported here instead of being left to be found on a board.
    ///
    /// INVARIANT: broken means exactly what a write means by it, because it
    /// is the same check. The two cannot come to disagree about whether a
    /// stored body still holds up.
    pub(crate) async fn dependents_state(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Vec<Dependent>, CustomError> {
        let holders = self.holders_of(kind, name).await?;
        let mut state = Vec::with_capacity(holders.len());

        for holder in holders {
            let refused = kinds::check(
                holder.kind,
                &holder.body,
                self.definitions,
                self.datasets,
                self.over,
            )
            .await
            .err();

            state.push(Dependent {
                reference: Reference::new(holder.kind, holder.name.into_string()),
                broken: refused.map(|error| error.to_string()),
            });
        }

        Ok(state)
    }

    pub(crate) async fn dependents_of(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Vec<Reference>, CustomError> {
        let holders = self.holders_of(kind, name).await?;

        Ok(holders
            .into_iter()
            .map(|holder| Reference::new(holder.kind, holder.name.into_string()))
            .collect())
    }

    /// Gives a definition a name nobody holds, and points everything that drew
    /// it at the new one.
    ///
    /// A name is the only handle a widget has on its metric, and a board on
    /// its widgets, so the new name, the removal of the old, and every
    /// rewritten dependent are one transaction. Taking the new name is a write
    /// that refuses to replace anything, so a name another writer takes while
    /// this one runs refuses the rename rather than overwriting them.
    pub(crate) async fn rename(
        &self,
        kind: DefinitionKind,
        from: &DefinitionName,
        to: &DefinitionName,
    ) -> Result<Vec<String>, CustomError> {
        let body = self.get(kind, from).await?;

        if to == from {
            return Ok(Vec::new());
        }

        let mut changes = vec![
            Change::Create(kind, to.clone(), body),
            Change::Delete(kind, from.clone()),
        ];
        let mut rewritten = Vec::new();

        for holder in self.holders_of(kind, from).await? {
            let pointed_at =
                kinds::rename_reference(holder.kind, holder.body, from.as_str(), to.as_str());

            rewritten.push(holder.name.as_str().to_owned());
            changes.push(Change::Put(holder.kind, holder.name, pointed_at));
        }

        self.definitions
            .apply(&changes)
            .await
            .map_err(CustomError::Store)?;

        Ok(rewritten)
    }

    /// Every stored definition naming this one, with the body that names it.
    ///
    /// The body comes back because both callers need it: one to say what is
    /// still in use, the other to rewrite it.
    async fn holders_of(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Vec<Definition>, CustomError> {
        let mut holders = Vec::new();

        for holder in kinds::referred_to_by(kind) {
            holders.extend(self.holders_among(*holder, kind, name).await?);
        }

        Ok(holders)
    }

    /// The definitions of one kind that name `name`, with their bodies.
    async fn holders_among(
        &self,
        holder: DefinitionKind,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Vec<Definition>, CustomError> {
        let mut found = Vec::new();

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

            if names(holder, &body, kind, name) {
                found.push(Definition::new(holder, parsed, body));
            }
        }

        Ok(found)
    }
}

/// Whether this body names that definition.
fn names(
    holder: DefinitionKind,
    body: &Value,
    kind: DefinitionKind,
    name: &DefinitionName,
) -> bool {
    kinds::refers_to(holder, body)
        .into_iter()
        .any(|reference| reference.kind == kind && reference.name == name.as_str())
}
