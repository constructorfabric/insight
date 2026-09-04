//! The folds a query computes over each group.
//!
//! INVARIANT: a fold over nothing observed reports NULL, and `count` — the one
//! fold whose zero is a real observation — reports 0.

use crate::domain::query::plan::{FoldFn, PlannedAggregate, PlannedFold, QueryPlan, column_alias};

use super::filters;
use super::params::QueryParam;

pub fn select_terms(plan: &QueryPlan<'_>, params: &mut Vec<QueryParam>) -> Vec<String> {
    let offset = plan.group_by.len();
    plan.aggregates
        .iter()
        .enumerate()
        .map(|(index, aggregate)| {
            let expr = fold_expr(aggregate, params);
            format!("{expr} AS {}", column_alias(offset + index))
        })
        .collect()
}

fn fold_expr(aggregate: &PlannedAggregate<'_>, params: &mut Vec<QueryParam>) -> String {
    let (function, operand) = match &aggregate.fold {
        PlannedFold::Rows => ("count", None),
        PlannedFold::Values {
            function,
            measurable,
        } => (function_name(*function), Some(measurable.field.as_str())),
    };
    // A count's zero is a real observation; every other fold saw nothing.
    let empty = if operand.is_none() { "" } else { "OrNull" };

    let Some(filter) = &aggregate.filter else {
        return match operand {
            None => format!("{function}{empty}()"),
            Some(column) => format!("{function}{empty}({column})"),
        };
    };

    // SAFETY: the collapse makes a row the filter is unknown of a definite
    // non-match rather than a row that silently leaves the fold.
    let condition = format!("coalesce({}, 0)", filters::render(filter, params));
    match operand {
        None => format!("{function}If{empty}({condition})"),
        Some(column) => format!("{function}If{empty}({column}, {condition})"),
    }
}

fn function_name(function: FoldFn) -> &'static str {
    match function {
        FoldFn::Sum => "sum",
        FoldFn::Avg => "avg",
        FoldFn::Min => "min",
        FoldFn::Max => "max",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::domain::query::contract::dto::ScalarDto;
    use crate::domain::query::datasets::declaration::{Dimension, Measurable};
    use crate::domain::query::plan::{FilterTarget, PlannedFilter, PlannedTest};

    fn measurable() -> Measurable {
        Measurable {
            field: "lines_added".to_owned(),
        }
    }

    fn dimension() -> Dimension {
        Dimension {
            field: "source".to_owned(),
            label_field: None,
            absent_value: None,
        }
    }

    fn on_github(dimension: &Dimension) -> PlannedFilter<'_> {
        PlannedFilter {
            target: FilterTarget::Dimension(dimension),
            test: PlannedTest::Eq(ScalarDto::Text("github".to_owned())),
        }
    }

    fn rendered(aggregate: &PlannedAggregate<'_>) -> (String, Vec<QueryParam>) {
        let mut params = Vec::new();
        let sql = fold_expr(aggregate, &mut params);
        (sql, params)
    }

    fn fold<'d>(fold: PlannedFold<'d>, filter: Option<PlannedFilter<'d>>) -> PlannedAggregate<'d> {
        PlannedAggregate {
            name: "value".to_owned(),
            fold,
            filter,
        }
    }

    fn values(function: FoldFn, measurable: &Measurable) -> PlannedFold<'_> {
        PlannedFold::Values {
            function,
            measurable,
        }
    }

    #[test]
    fn every_fold_over_no_observation_reports_null_except_a_count() {
        let measurable = measurable();
        let cases = [
            (PlannedFold::Rows, "count()"),
            (values(FoldFn::Sum, &measurable), "sumOrNull(lines_added)"),
            (values(FoldFn::Avg, &measurable), "avgOrNull(lines_added)"),
            (values(FoldFn::Min, &measurable), "minOrNull(lines_added)"),
            (values(FoldFn::Max, &measurable), "maxOrNull(lines_added)"),
        ];

        for (planned, expected) in cases {
            let label = format!("{planned:?}");
            let (sql, params) = rendered(&fold(planned, None));
            assert_eq!(sql, expected, "{label}");
            assert!(params.is_empty(), "{label}");
        }
    }

    #[test]
    fn a_conditional_fold_treats_an_unknown_condition_as_a_non_match() {
        let dimension = dimension();
        let measurable = measurable();

        let (sql, params) = rendered(&fold(
            values(FoldFn::Sum, &measurable),
            Some(on_github(&dimension)),
        ));

        assert_eq!(
            sql,
            "sumIfOrNull(lines_added, coalesce(toString(source) = ?, 0))"
        );
        assert_eq!(params, vec![QueryParam::Text("github".to_owned())]);
        assert_eq!(sql.matches('?').count(), params.len());
    }

    #[test]
    fn a_conditional_count_folds_rows_and_reads_no_column() {
        let dimension = dimension();

        let (sql, _) = rendered(&fold(PlannedFold::Rows, Some(on_github(&dimension))));

        assert_eq!(sql, "countIf(coalesce(toString(source) = ?, 0))");
    }
}
