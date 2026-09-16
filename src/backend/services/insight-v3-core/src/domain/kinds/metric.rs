//! What a stored metric admits, and what it names.

use serde_json::Value;

use super::{KindError, Reference};
use crate::domain::query::metric_query::MetricQuery;

/// What a stored metric says about time, checked before it is stored.
///
/// A body that is not a metric at all is left alone here — it is refused when
/// run, and refusing it here would be a change of behaviour this move does not
/// make.
pub(crate) fn check(body: &Value) -> Result<(), KindError> {
    let Ok(metric) = serde_json::from_value::<MetricQuery>(body.clone()) else {
        return Ok(());
    };

    metric.check_window().map_err(KindError::Compile)
}

/// A metric names no other definition: it reads a relation, not a definition.
pub(crate) fn refers_to(_body: &Value) -> Vec<Reference> {
    Vec::new()
}
