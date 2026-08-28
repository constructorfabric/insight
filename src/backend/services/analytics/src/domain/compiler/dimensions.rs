//! How a requested dimension becomes columns of a result row.
//!
//! INVARIANT: position, not key, is what the row decoder reads, and an absent
//! value groups under a visible sentinel rather than under NULL.

use std::fmt::Write;

use crate::domain::definitions::definition::{DimensionBinding, MeasureDefinition};

use super::error::CompileError;
use super::sql::{dimension_binding, label_field};

pub(super) const UNKNOWN_DIMENSION_VALUE: &str = "__unknown__";
pub(super) const UNKNOWN_DIMENSION_LABEL: &str = "Unknown";

/// The array columns a materialized row carries its dimension tuples in.
pub(super) const CACHED_KEYS: &str = "dimensions.1";
pub(super) const CACHED_VALUES: &str = "dimensions.2";
const CACHED_LABELS: &str = "dimensions.3";

/// The date column standing where a dataset read takes the measure's event time.
pub(super) const CACHED_DATE: &str = "metric_date";

/// The column pair one requested dimension occupies in a result row.
pub(crate) fn dimension_aliases(index: usize) -> (String, String) {
    (format!("dim_{index}_value"), format!("dim_{index}_label"))
}

/// Where a read takes a requested dimension's value and label from.
pub(super) enum DimensionSource<'a> {
    /// The dataset's own columns, as the measure binds them.
    Row(&'a MeasureDefinition),
    /// The dimension tuples the measure's materialized rows carry.
    Cached(&'a MeasureDefinition),
}

impl DimensionSource<'_> {
    /// The value and label expressions one requested key resolves to.
    fn exprs(&self, key: &str) -> Result<(String, String), CompileError> {
        match self {
            Self::Row(measure) => {
                let binding = dimension_binding(measure, key)?;
                Ok((dimension_value_expr(binding), dimension_label_expr(binding)))
            }
            Self::Cached(measure) => {
                dimension_binding(measure, key)?;
                Ok((cached_value_expr(key), cached_label_expr(key)))
            }
        }
    }

    /// The date a group's label is picked by its latest row on.
    fn event_date(&self) -> String {
        match self {
            Self::Row(measure) => format!("toDate({})", measure.event_time),
            Self::Cached(_) => CACHED_DATE.to_owned(),
        }
    }
}

pub(super) fn dimension_value_expr(binding: &DimensionBinding) -> String {
    format!(
        "coalesce(toString({}), '{UNKNOWN_DIMENSION_VALUE}')",
        binding.value_field
    )
}

pub(super) fn dimension_label_expr(binding: &DimensionBinding) -> String {
    format!(
        "coalesce(toString({}), '{UNKNOWN_DIMENSION_LABEL}')",
        label_field(binding)
    )
}

// SAFETY: only a key the measure declares reaches here, so what is written into
// the statement is an authored key rather than a caller's string.
pub(super) fn cached_value_expr(key: &str) -> String {
    cached_lookup(key, CACHED_VALUES, UNKNOWN_DIMENSION_VALUE)
}

pub(super) fn cached_label_expr(key: &str) -> String {
    cached_lookup(key, CACHED_LABELS, UNKNOWN_DIMENSION_LABEL)
}

fn cached_lookup(key: &str, column: &str, absent: &str) -> String {
    let index = format!("indexOf({CACHED_KEYS}, '{key}')");
    format!("if({index} = 0, '{absent}', coalesce({column}[{index}], '{absent}'))")
}

/// Value and label both group, so each row's label belongs to its own value.
pub(super) fn dimension_select_group(
    source: &DimensionSource<'_>,
    keys: &[String],
) -> Result<(String, String), CompileError> {
    let mut select = String::new();
    let mut group = Vec::with_capacity(keys.len() * 2);

    for (index, key) in keys.iter().enumerate() {
        let (value, label) = source.exprs(key)?;
        let (value_alias, label_alias) = dimension_aliases(index);
        let _ = write!(
            select,
            "    {value} AS {value_alias},\n    {label} AS {label_alias},\n"
        );
        group.push(value_alias);
        group.push(label_alias);
    }

    Ok((select, group.join(", ")))
}

/// The label is not a group key, so each group reports the label its latest
/// row carries, broken by the label itself so the pick is total.
pub(super) fn combined_split_dimension_select_group(
    source: &DimensionSource<'_>,
    keys: &[String],
) -> Result<(String, String), CompileError> {
    let mut select = String::new();
    let mut group = Vec::with_capacity(keys.len());
    let event_date = source.event_date();

    for (index, key) in keys.iter().enumerate() {
        let (value, label) = source.exprs(key)?;
        let (value_alias, label_alias) = dimension_aliases(index);
        let _ = write!(
            select,
            "    {value} AS {value_alias},\n    argMax({label}, tuple({event_date}, {label})) AS {label_alias},\n"
        );
        group.push(value_alias);
    }

    Ok((select, group.join(", ")))
}
