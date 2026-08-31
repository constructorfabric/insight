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

/// The column pair one requested dimension occupies in a result row.
pub(crate) fn dimension_aliases(index: usize) -> (String, String) {
    (format!("dim_{index}_value"), format!("dim_{index}_label"))
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

/// Value and label both group, so each row's label belongs to its own value.
pub(super) fn dimension_select_group(
    measure: &MeasureDefinition,
    keys: &[String],
) -> Result<(String, String), CompileError> {
    let mut select = String::new();
    let mut group = Vec::with_capacity(keys.len() * 2);

    for (index, key) in keys.iter().enumerate() {
        let binding = dimension_binding(measure, key)?;
        let (value_alias, label_alias) = dimension_aliases(index);
        let _ = write!(
            select,
            "    {} AS {value_alias},\n    {} AS {label_alias},\n",
            dimension_value_expr(binding),
            dimension_label_expr(binding)
        );
        group.push(value_alias);
        group.push(label_alias);
    }

    Ok((select, group.join(", ")))
}

/// The label is not a group key, so each group reports the label its latest
/// row carries, broken by the label itself so the pick is total.
pub(super) fn combined_split_dimension_select_group(
    measure: &MeasureDefinition,
    keys: &[String],
) -> Result<(String, String), CompileError> {
    let mut select = String::new();
    let mut group = Vec::with_capacity(keys.len());

    for (index, key) in keys.iter().enumerate() {
        let binding = dimension_binding(measure, key)?;
        let (value_alias, label_alias) = dimension_aliases(index);
        let label = dimension_label_expr(binding);
        let _ = write!(
            select,
            "    {} AS {value_alias},\n    argMax({label}, tuple(toDate({}), {label})) AS {label_alias},\n",
            dimension_value_expr(binding),
            measure.event_time
        );
        group.push(value_alias);
    }

    Ok((select, group.join(", ")))
}
