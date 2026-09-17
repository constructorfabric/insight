//! What a stored metric admits, and what it names.

pub(crate) mod answerable;

use serde_json::Value;

use super::{KindError, Reference};
use crate::domain::datasets::{self, Datasets};
use crate::domain::query::metric_query::{MetricQuery, MetricQueryError};

/// Whether a stored metric can be read back and run.
///
/// A body stored under a name is what every widget drawing that name will
/// get, so one that cannot be read as a metric at all is refused here rather
/// than at each of them. What the metric asks of its dataset is settled here
/// too: the declaration is what says whether a field exists, what type it
/// holds, and so whether the question has an answer.
pub(crate) async fn check(body: &Value, datasets: &dyn Datasets) -> Result<(), KindError> {
    let metric: MetricQuery = serde_json::from_value(body.clone()).map_err(KindError::Body)?;

    let Some(named) = metric.dataset() else {
        return Err(KindError::Compile(MetricQueryError::NoDataset));
    };
    if metric.addresses_a_relation() {
        return Err(KindError::Compile(MetricQueryError::AddressesARelation));
    }
    metric.check_window().map_err(KindError::Compile)?;

    let Some(ready) = datasets::ready(datasets, named).await else {
        return Err(KindError::DatasetNotReady(named.to_owned()));
    };

    let violations = answerable::check(&metric, &ready.declaration);
    if violations.is_empty() {
        return Ok(());
    }

    Err(KindError::Unanswerable(violations))
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
