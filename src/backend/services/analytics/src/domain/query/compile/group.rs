//! How a group axis becomes a column of the answer.
//!
//! INVARIANT: the answer and a filter both read a dimension's rendered value,
//! never the raw column.

use crate::domain::query::datasets::declaration::Dimension;
use crate::domain::query::plan::{PlannedAxis, QueryPlan, column_alias};

use super::time::bucket_expr;

/// What the answer reports for one dimension; a group is never NULL.
pub fn dimension_expr(dimension: &Dimension) -> String {
    let value = format!("toString({})", dimension.field);
    match &dimension.absent_value {
        // SAFETY: the sentinel comes from the declaration, not from a caller.
        Some(absent) => format!("coalesce({value}, '{}')", escape_literal(absent)),
        None => value,
    }
}

pub fn select_terms(plan: &QueryPlan<'_>) -> Vec<String> {
    plan.group_by
        .iter()
        .enumerate()
        .map(|(index, axis)| {
            let expr = match axis {
                PlannedAxis::Dimension(dimension) => dimension_expr(dimension),
                // INVARIANT: validation admits a time axis only beside a grain.
                PlannedAxis::Time => plan
                    .time
                    .grain
                    .map(|grain| bucket_expr(plan.time.field, grain))
                    .unwrap_or_default(),
            };
            format!("{expr} AS {}", column_alias(index))
        })
        .collect()
}

pub fn group_aliases(plan: &QueryPlan<'_>) -> Vec<String> {
    (0..plan.group_by.len()).map(column_alias).collect()
}

fn escape_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn dimension(absent_value: Option<&str>) -> Dimension {
        Dimension {
            field: "repository".to_owned(),
            label_field: None,
            absent_value: absent_value.map(str::to_owned),
        }
    }

    #[test]
    fn a_dimension_over_a_non_nullable_column_reports_the_column_as_text() {
        assert_eq!(dimension_expr(&dimension(None)), "toString(repository)");
    }

    #[test]
    fn a_nullable_dimension_reports_its_absent_rows_under_the_declared_value() {
        assert_eq!(
            dimension_expr(&dimension(Some("__unknown__"))),
            "coalesce(toString(repository), '__unknown__')"
        );
    }

    #[test]
    fn a_quote_in_a_declared_value_cannot_end_the_literal() {
        assert_eq!(
            dimension_expr(&dimension(Some("it's"))),
            r"coalesce(toString(repository), 'it\'s')"
        );
    }
}
