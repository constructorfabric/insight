//! One module per kind of definition, holding what that kind admits.
//!
//! Everything a kind knows about itself lives in its own module; this root
//! dispatches to them. INVARIANT: every dispatch matches exhaustively, so a
//! kind added to [`DefinitionKind`] cannot be stored unchecked or leave its
//! references unseen.

pub(crate) mod dashboard;
pub(crate) mod dataset;
pub(crate) mod metric;
pub(crate) mod widget;

use serde_json::Value;
use thiserror::Error;

use widget::WidgetError;

use crate::domain::datasets::Datasets;
use crate::domain::definition::{DefinitionKind, DefinitionStoreError, Lookup};
use crate::domain::query::metric_query::MetricQueryError;
use crate::domain::query::time_window::WindowError;
use crate::domain::violation::Violation;

/// One definition naming another by id: a widget naming its metric, a board
/// naming a widget it draws.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Reference {
    pub(crate) kind: DefinitionKind,
    pub(crate) id: String,
}

impl Reference {
    pub(crate) fn new(kind: DefinitionKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }
}

/// Why a body cannot be stored under its kind.
#[derive(Debug, Error)]
pub(crate) enum KindError {
    #[error(transparent)]
    Widget(WidgetError),
    #[error("definition body is not valid: {0}")]
    Body(serde_json::Error),
    #[error(transparent)]
    Compile(MetricQueryError),
    #[error("dashboard time range: {0}")]
    Range(WindowError),
    #[error(transparent)]
    Store(DefinitionStoreError),
    #[error("no dataset named `{0}` is ready to be read")]
    DatasetNotReady(String),
    #[error(transparent)]
    Datasets(crate::domain::datasets::DatasetStoreError),
    #[error("this dataset cannot answer that metric - {}", crate::domain::violation::said(.0))]
    Unanswerable(Vec<Violation>),
}

/// Checks a body against the rules of its kind, before it is stored.
///
/// A lookup is passed because a kind may need what another definition holds:
/// a widget draws its metric's columns, so it reads that metric. The datasets
/// are passed for the same reason: a metric reads one, and what that dataset
/// declares is what decides whether the metric has an answer.
pub(crate) async fn check(
    kind: DefinitionKind,
    body: &Value,
    definitions: &dyn Lookup,
    datasets: &dyn Datasets,
    over: metric::CompileAgainst<'_>,
) -> Result<(), KindError> {
    match kind {
        DefinitionKind::Metric => metric::check(body, datasets, over).await,
        DefinitionKind::Widget => widget::check(body, definitions, datasets).await,
        DefinitionKind::Dashboard => dashboard::check(body),
    }
}

/// Every definition this body names, whatever kind it is.
pub(crate) fn refers_to(kind: DefinitionKind, body: &Value) -> Vec<Reference> {
    match kind {
        DefinitionKind::Metric => metric::refers_to(body),
        DefinitionKind::Widget => widget::refers_to(body),
        DefinitionKind::Dashboard => dashboard::refers_to(body),
    }
}

/// The same body, with every reference to `from` now naming `to`.
///
/// A chat proposal's items name each other by a key local to the proposal,
/// which becomes the id each is stored under.
pub(crate) fn rename_reference(kind: DefinitionKind, body: Value, from: &str, to: &str) -> Value {
    match kind {
        DefinitionKind::Metric => body,
        DefinitionKind::Widget => widget::rename_reference(body, from, to),
        DefinitionKind::Dashboard => dashboard::rename_reference(body, from, to),
    }
}

/// The same body, with each reference naming a pair's `from` now naming its
/// `to`, all in one pass: a `to` that is also another pair's `from` is not
/// renamed a second time.
pub(crate) fn rename_references(
    kind: DefinitionKind,
    body: Value,
    pairs: &[(&str, &str)],
) -> Value {
    // INVARIANT: a placeholder holds a control character, which no label and
    // no id may, so it can never be mistaken for a reference.
    let placeholder = |place: usize| format!("\u{1}{place}");

    let parked = pairs
        .iter()
        .enumerate()
        .fold(body, |body, (place, (from, _))| {
            rename_reference(kind, body, from, &placeholder(place))
        });

    pairs
        .iter()
        .enumerate()
        .fold(parked, |body, (place, (_, to))| {
            rename_reference(kind, body, &placeholder(place), to)
        })
}

/// The kinds whose bodies can name this one, and so the only kinds worth
/// reading when asking what still depends on it.
pub(crate) fn referred_to_by(kind: DefinitionKind) -> &'static [DefinitionKind] {
    match kind {
        DefinitionKind::Metric => &[DefinitionKind::Widget],
        DefinitionKind::Widget => &[DefinitionKind::Dashboard],
        DefinitionKind::Dashboard => &[],
    }
}

#[cfg(test)]
mod tests;
