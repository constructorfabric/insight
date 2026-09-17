//! What a stored metric admits, and what it names.

pub(crate) mod answerable;

use serde_json::Value;

use super::{KindError, Reference};
use crate::domain::query::metric_query::MetricQuery;

/// Whether a stored metric can be read back and run.
///
/// A body stored under a name is what every widget drawing that name will
/// get, so one that cannot be read as a metric at all is refused here rather
/// than at each of them.
pub(crate) fn check(body: &Value) -> Result<(), KindError> {
    let metric: MetricQuery = serde_json::from_value(body.clone()).map_err(KindError::Body)?;

    metric.check_window().map_err(KindError::Compile)
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
