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

const A_WINDOW: Window = Window::Requested {
    bounds: Bounds::Finite {
        from: chrono::DateTime::UNIX_EPOCH,
        to: chrono::DateTime::UNIX_EPOCH,
    },
    grain: Some(Grain::Day),
};

pub(crate) async fn check(
    body: &Value,
    datasets: &dyn Datasets,
    over: CompileAgainst<'_>,
) -> Result<(), KindError> {
    let metric: MetricQuery = serde_json::from_value(body.clone()).map_err(KindError::Body)?;

    let Some(named) = metric.dataset() else {
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct CompileAgainst<'a> {
    pub(crate) database: &'a str,
    pub(crate) people: &'a People,
}

fn compiles(
    metric: &MetricQuery,
    people: &People,
    over: Option<Over<'_>>,
    dated: bool,
) -> Result<(), KindError> {
    metric
        .compile_window(people, &Window::legacy(), TableEngine::Other, over)
        .map_err(KindError::Compile)?;

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

pub(crate) fn refers_to(_body: &Value) -> Vec<Reference> {
    Vec::new()
}

pub(crate) fn reads_dataset(body: &Value) -> Option<String> {
    body.get("dataset")
        .and_then(Value::as_str)
        .map(str::to_owned)
}
