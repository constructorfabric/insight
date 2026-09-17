//! What a stored metric admits, and what it names.

// Its own tests exercise it; the metric write path and the compiler that call
// it arrive in the steps after this one. `expect` rather than `allow`, so the
// marker fails once they do.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired up by the metric write path")
)]
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

/// A metric names no other definition: it reads a relation, not a definition.
pub(crate) fn refers_to(_body: &Value) -> Vec<Reference> {
    Vec::new()
}
