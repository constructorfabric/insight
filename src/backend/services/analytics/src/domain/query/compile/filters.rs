//! Renders one filter into a predicate and the values it binds.

use crate::domain::query::contract::dto::ScalarDto;
use crate::domain::query::plan::{CompareOp, FilterTarget, PlannedFilter, PlannedTest};

use super::group::dimension_expr;
use super::params::{QueryParam, placeholders};

pub fn render(filter: &PlannedFilter<'_>, params: &mut Vec<QueryParam>) -> String {
    let mut bind = |scalar: &ScalarDto| params.push(bound(filter, scalar));

    match &filter.test {
        // SAFETY: reads the raw column — a dimension's sentinel would answer
        // that the row always has a value.
        PlannedTest::NotNull => format!("{} IS NOT NULL", raw_column(filter)),
        PlannedTest::Eq(value) => {
            bind(value);
            format!("{} = ?", comparison_expr(filter))
        }
        PlannedTest::Compare(op, value) => {
            bind(value);
            format!("{} {} ?", comparison_expr(filter), operator(*op))
        }
        PlannedTest::In(values) => {
            for value in values {
                bind(value);
            }
            format!(
                "{} IN ({})",
                comparison_expr(filter),
                placeholders(values.len())
            )
        }
        PlannedTest::Between { low, high } => {
            bind(low);
            bind(high);
            let expr = comparison_expr(filter);
            format!("({expr} >= ? AND {expr} <= ?)")
        }
    }
}

fn comparison_expr(filter: &PlannedFilter<'_>) -> String {
    match filter.target {
        FilterTarget::Dimension(dimension) => dimension_expr(dimension),
        FilterTarget::Measurable(measurable) => measurable.field.clone(),
    }
}

fn raw_column<'f>(filter: &'f PlannedFilter<'_>) -> &'f str {
    match filter.target {
        FilterTarget::Dimension(dimension) => &dimension.field,
        FilterTarget::Measurable(measurable) => &measurable.field,
    }
}

fn bound(filter: &PlannedFilter<'_>, scalar: &ScalarDto) -> QueryParam {
    match filter.target {
        FilterTarget::Dimension(_) => QueryParam::text_of(scalar),
        FilterTarget::Measurable(_) => QueryParam::number_of(scalar),
    }
}

fn operator(op: CompareOp) -> &'static str {
    match op {
        CompareOp::Gt => ">",
        CompareOp::Gte => ">=",
        CompareOp::Lt => "<",
        CompareOp::Lte => "<=",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::domain::query::datasets::declaration::{Dimension, Measurable};

    fn dimension() -> Dimension {
        Dimension {
            field: "source_id".to_owned(),
            label_field: None,
            absent_value: Some("__unknown__".to_owned()),
        }
    }

    fn measurable() -> Measurable {
        Measurable {
            field: "lines_added".to_owned(),
        }
    }

    fn text(value: &str) -> ScalarDto {
        ScalarDto::Text(value.to_owned())
    }

    fn number(raw: &str) -> ScalarDto {
        ScalarDto::Number(raw.parse().expect("a JSON number"))
    }

    fn render_with(target: FilterTarget<'_>, test: PlannedTest) -> (String, Vec<QueryParam>) {
        let filter = PlannedFilter { target, test };
        let mut params = Vec::new();
        let sql = render(&filter, &mut params);
        (sql, params)
    }

    #[test]
    fn every_test_over_a_measurable_reads_the_numeric_column() {
        let measurable = measurable();
        let cases = [
            (PlannedTest::Eq(number("500")), "lines_added = ?", 1),
            (
                PlannedTest::Compare(CompareOp::Gt, number("500")),
                "lines_added > ?",
                1,
            ),
            (
                PlannedTest::Compare(CompareOp::Gte, number("500")),
                "lines_added >= ?",
                1,
            ),
            (
                PlannedTest::Compare(CompareOp::Lt, number("500")),
                "lines_added < ?",
                1,
            ),
            (
                PlannedTest::Compare(CompareOp::Lte, number("500")),
                "lines_added <= ?",
                1,
            ),
            (
                PlannedTest::In(vec![number("1"), number("2")]),
                "lines_added IN (?, ?)",
                2,
            ),
            (
                PlannedTest::Between {
                    low: number("1"),
                    high: number("9"),
                },
                "(lines_added >= ? AND lines_added <= ?)",
                2,
            ),
            (PlannedTest::NotNull, "lines_added IS NOT NULL", 0),
        ];

        for (test, expected, bound) in cases {
            let label = format!("{test:?}");
            let (sql, params) = render_with(FilterTarget::Measurable(&measurable), test);
            assert_eq!(sql, expected, "{label}");
            assert_eq!(params.len(), bound, "{label}");
            assert_eq!(sql.matches('?').count(), params.len(), "{label}");
        }
    }

    #[test]
    fn a_dimension_filter_compares_against_the_value_the_answer_reports() {
        let dimension = dimension();
        let (sql, params) = render_with(
            FilterTarget::Dimension(&dimension),
            PlannedTest::Eq(text("__unknown__")),
        );

        assert_eq!(sql, "coalesce(toString(source_id), '__unknown__') = ?");
        assert_eq!(params, vec![QueryParam::Text("__unknown__".to_owned())]);
    }

    #[test]
    fn not_null_over_a_dimension_asks_the_raw_column_the_sentinel_would_hide() {
        let dimension = dimension();
        let (sql, params) = render_with(FilterTarget::Dimension(&dimension), PlannedTest::NotNull);

        assert_eq!(sql, "source_id IS NOT NULL");
        assert!(params.is_empty());
    }

    #[test]
    fn a_measurable_value_binds_as_a_number_and_a_dimension_value_as_text() {
        let measurable = measurable();
        let (_, numeric) = render_with(
            FilterTarget::Measurable(&measurable),
            PlannedTest::Compare(CompareOp::Gte, number("500")),
        );
        assert_eq!(numeric, vec![QueryParam::Int(500)]);

        let dimension = dimension();
        let (_, textual) = render_with(
            FilterTarget::Dimension(&dimension),
            PlannedTest::Eq(number("500")),
        );
        assert_eq!(textual, vec![QueryParam::Text("500".to_owned())]);
    }
}
