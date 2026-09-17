//! What a stored metric admits, and what it names.

pub(crate) mod answerable;

use serde_json::Value;

use super::{KindError, Reference};
use crate::domain::datasets::{self, Datasets, Ready};
use crate::domain::kinds::metric::answerable::effective_clock;
use crate::domain::query::metric_query::over::Over;
use crate::domain::query::metric_query::{MetricQuery, MetricQueryError, People};
use crate::domain::query::time_window::{Bounds, Grain, Window};
use crate::domain::violation::{Reason, Violation};
use crate::store::catalog::TableEngine;

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
        return Err(KindError::Compile(MetricQueryError::NoDataset));
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

    compiles(&metric, &ready, over)
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
fn compiles(
    metric: &MetricQuery,
    ready: &Ready,
    against: CompileAgainst<'_>,
) -> Result<(), KindError> {
    let over = Over {
        declaration: &ready.declaration,
        database: against.database,
        table: &ready.table,
    };

    metric
        .compile_over(against.people, &Window::legacy(), over)
        .map_err(KindError::Compile)?;

    // A metric with a date answers windows too, and windowing is where the
    // bucket, the cap and the undated count come in.
    if effective_clock(metric, &ready.declaration).is_some() {
        metric
            .compile_over(against.people, &A_WINDOW, over)
            .map_err(KindError::Compile)?;
        metric
            .undated_query(TableEngine::Other, Some(over))
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
