//! What a stored metric admits, and what it names.

pub(crate) mod answerable;

use serde_json::Value;

use super::{KindError, Reference};
use crate::domain::datasets::{self, Datasets};
use crate::domain::kinds::metric::answerable::effective_clock;
use crate::domain::query::metric_query::TableEngine;
use crate::domain::query::metric_query::over::Over;
use crate::domain::query::metric_query::{MetricQuery, MetricQueryError, People};
use crate::domain::query::time_window::{Bounds, Grain, Window};
use crate::domain::violation::{Reason, Violation};

/// The window a dry compilation uses: bucketed, so the bucket and the cap are
/// exercised, and empty, so no cap can refuse it for its width.
const A_WINDOW: Window = Window::Requested {
    bounds: Bounds::Finite {
        from: chrono::DateTime::UNIX_EPOCH,
        to: chrono::DateTime::UNIX_EPOCH,
    },
    grain: Some(Grain::Day),
};

/// Whether a stored metric can be read back and run.
///
/// A body stored under a name is what every widget drawing that name will
/// get, so one that cannot be read as a metric at all is refused here rather
/// than at each of them. What the metric asks of its dataset is settled here
/// too: the declaration is what says whether a field exists, what type it
/// holds, and so whether the question has an answer.
pub(crate) async fn check(
    body: &Value,
    datasets: &dyn Datasets,
    over: CompileAgainst<'_>,
) -> Result<(), KindError> {
    let metric: MetricQuery = serde_json::from_value(body.clone()).map_err(KindError::Body)?;

    let Some(named) = metric.dataset() else {
        names_a_declared_field(&metric)?;
        let dated = metric.has_clock().map_err(KindError::Compile)?;
        return compiles(&metric, over.people, None, dated);
    };
    if metric.addresses_a_relation() {
        return Err(KindError::Compile(MetricQueryError::AddressesARelation));
    }
    reaches_past_the_declaration(&metric)?;
    metric.check_window().map_err(KindError::Compile)?;

    let Some(ready) = datasets::ready(datasets, named)
        .await
        .map_err(KindError::Datasets)?
    else {
        return Err(KindError::DatasetNotReady(named.to_owned()));
    };

    let violations = answerable::check(&metric, &ready.declaration);
    if !violations.is_empty() {
        return Err(KindError::Unanswerable(violations));
    }

    let relation = Over {
        declaration: &ready.declaration,
        database: over.database,
        table: &ready.table,
    };
    let dated = effective_clock(&metric, &ready.declaration).is_some();

    compiles(&metric, over.people, Some(relation), dated)
}

/// What a dry compilation needs beside the dataset: where the records are
/// kept, and where a person's name is resolved from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CompileAgainst<'a> {
    pub(crate) database: &'a str,
    pub(crate) people: &'a People,
}

/// Whether the query this metric describes can actually be built.
///
/// INVARIANT: every rule the compiler enforces is enforced here, by running
/// it. A metric that is stored and then refuses to run is a definition no
/// reader can act on, and the place to say so is the write.
///
/// Over a warehouse table the engine is not known until it runs; it only
/// decides whether the read adds `FINAL`, so `Other` compiles the same query.
fn compiles(
    metric: &MetricQuery,
    people: &People,
    over: Option<Over<'_>>,
    dated: bool,
) -> Result<(), KindError> {
    metric
        .compile_window(people, &Window::legacy(), TableEngine::Other, over)
        .map_err(KindError::Compile)?;

    // A metric with a date answers windows too, and windowing is where the
    // bucket, the cap and the undated count come in.
    if dated {
        metric
            .compile_window(people, &A_WINDOW, TableEngine::Other, over)
            .map_err(KindError::Compile)?;
        metric
            .undated_query(TableEngine::Other, over)
            .map_err(KindError::Compile)?;
    }

    Ok(())
}

/// Every place the metric reaches into a record instead of naming a field.
fn reaches_past_the_declaration(metric: &MetricQuery) -> Result<(), KindError> {
    let reached = metric.physical_references();
    if reached.is_empty() {
        return Ok(());
    }

    Err(KindError::Unanswerable(
        reached
            .into_iter()
            .map(|(at, key)| {
                Violation::new(
                    at,
                    Reason::NotAdmissible,
                    format!(
                        "a metric over a dataset names a declared `field`; \
                         `{key}` reads the record itself"
                    ),
                )
            })
            .collect(),
    ))
}

/// Every place a metric over a table names a field of a dataset.
///
/// The mirror of the check above. Without it the read falls through to "this
/// field names neither a column nor a json key", which names the column the
/// metric produces and says nothing about the `field` that is the reason.
fn names_a_declared_field(metric: &MetricQuery) -> Result<(), KindError> {
    let named = metric.field_references();
    if named.is_empty() {
        return Ok(());
    }

    Err(KindError::Unanswerable(
        named
            .into_iter()
            .map(|reference| {
                Violation::new(
                    reference.at,
                    Reason::NotAdmissible,
                    "a metric over a table names a `column`, or a `json` key inside one; \
                     `field` names a field of a dataset"
                        .to_owned(),
                )
            })
            .collect(),
    ))
}

/// A metric names no other definition: it reads a dataset, which is not one.
pub(crate) fn refers_to(_body: &Value) -> Vec<Reference> {
    Vec::new()
}

/// The dataset this metric reads, when its body names one readably.
///
/// Datasets are not definitions of the same kind — they own a table of records
/// and a lifecycle — so a metric's hold on one is not in the reference graph
/// the other kinds share.
pub(crate) fn reads_dataset(body: &Value) -> Option<String> {
    body.get("dataset")
        .and_then(Value::as_str)
        .map(str::to_owned)
}
