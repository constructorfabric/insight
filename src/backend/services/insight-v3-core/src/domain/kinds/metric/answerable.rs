//! Whether a dataset can answer the metric written over it.
//!
//! A metric names two kinds of thing. A **field reference** names a field the
//! dataset declares, and appears as the source of each selected field, of each
//! condition and of the clock. An **output name** names a column the metric
//! produces, and appears in its grouping and its ordering. Only the first kind
//! is the dataset's to answer for.

use serde::Serialize;

use crate::domain::kinds::dataset::declaration::{BUCKET_COLUMN, Declaration, FieldType};
use crate::domain::query::metric_query::{Aggregate, MetricQuery, Reference, Used};
use crate::domain::violation::{Reason, Violation};

/// Every reason this dataset cannot answer this metric, reported together.
pub(crate) fn check(metric: &MetricQuery, declaration: &Declaration) -> Vec<Violation> {
    let mut violations = Vec::new();

    for reference in metric.field_references() {
        check_reference(&reference, declaration, &mut violations);
    }

    let produced = metric.output_names();
    for (at, named) in metric.output_references() {
        if named != BUCKET_COLUMN && !produced.contains(&named) {
            violations.push(Violation::new(
                at,
                Reason::Unknown,
                format!(
                    "`{named}` is not a column this metric produces; it produces {}",
                    listed(&produced)
                ),
            ));
        }
    }

    violations
}

/// The field a window selects by, and where it was decided.
///
/// A metric that names none inherits the dataset's own; a dataset that marks
/// none leaves the metric answering every record, whatever window is picked.
pub(crate) fn effective_clock<'a>(
    metric: &'a MetricQuery,
    declaration: &'a Declaration,
) -> Option<(&'a str, ClockSource)> {
    if let Some(named) = metric.own_clock() {
        return Some((named, ClockSource::Metric));
    }

    declaration
        .default_clock()
        .map(|field| (field.name.as_str(), ClockSource::Dataset))
}

/// Who decided which field a window selects by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ClockSource {
    Metric,
    Dataset,
}

/// The clock a window over this metric selects by, as a reader is told it.
///
/// A metric over a dataset may name no clock and still be windowed, so its
/// body does not say whether a card follows the board's window. This does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct EffectiveClock {
    pub(crate) field: String,
    /// Whether the metric named this field or inherited it.
    pub(crate) from: ClockSource,
}

impl EffectiveClock {
    pub(crate) fn of(metric: &MetricQuery, declaration: &Declaration) -> Option<Self> {
        effective_clock(metric, declaration).map(|(field, from)| Self {
            field: field.to_owned(),
            from,
        })
    }
}

fn check_reference(
    reference: &Reference<'_>,
    declaration: &Declaration,
    into: &mut Vec<Violation>,
) {
    let Some(declared) = declaration.field(reference.field) else {
        into.push(Violation::new(
            reference.at.clone(),
            Reason::Unknown,
            format!(
                "`{}` is not a field of this dataset; it declares {}",
                reference.field,
                listed(&declaration.field_names())
            ),
        ));
        return;
    };

    // Grouping, filtering, counting, the smallest and the largest are open to
    // every type, so a numeric identifier groups as readily as a name.
    match &reference.used {
        Used::Selected(Some(Aggregate::Arithmetic)) if !declared.r#type.is_numeric() => {
            into.push(Violation::new(
                reference.at.clone(),
                Reason::NotAdmissible,
                format!(
                    "`{}` is declared {} and only a number can be summed or averaged",
                    reference.field,
                    declared.r#type.as_str()
                ),
            ));
        }
        Used::Clock if declared.r#type != FieldType::Datetime => {
            into.push(Violation::new(
                reference.at.clone(),
                Reason::NotAdmissible,
                format!(
                    "`{}` is declared {} and a window selects by a datetime",
                    reference.field,
                    declared.r#type.as_str()
                ),
            ));
        }
        Used::Compared(value) if !holds(declared.r#type, value) => {
            into.push(Violation::new(
                reference.at.clone(),
                Reason::Malformed,
                format!(
                    "`{}` is declared {} and the value compared against it is not one",
                    reference.field,
                    declared.r#type.as_str()
                ),
            ));
        }
        Used::Selected(_) | Used::Clock | Used::Compared(_) => {}
    }
}

/// Whether a value a metric compares against can be read as the declared type.
fn holds(declared: FieldType, value: &serde_json::Value) -> bool {
    match declared {
        FieldType::String | FieldType::Datetime => value.is_string(),
        FieldType::Int => value.is_i64() || value.is_u64(),
        FieldType::Float => value.is_number(),
        FieldType::Bool => value.is_boolean(),
    }
}

fn listed(names: &[&str]) -> String {
    if names.is_empty() {
        return "none".to_owned();
    }

    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests;
