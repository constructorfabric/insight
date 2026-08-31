use serde::{Deserialize, Serialize};

use crate::domain::metric_results::compiler::sql_string_literal;

use super::dto::MetricDrilldownColumnType;

/// `u64::MAX` is 20 digits. A longer run of digits is not a number worth
/// padding, and padding it would push a real value out of the field.
const NUMERIC_PAD_WIDTH: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricDrilldownSortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownSort {
    pub key: String,
    pub direction: MetricDrilldownSortDirection,
}

impl MetricDrilldownSort {
    /// What a page shows when the client names no order. A drilldown is opened
    /// to see what happened, and the end of the period is what happened last.
    pub(super) fn newest_first() -> Self {
        Self {
            key: DATE_KEY.to_owned(),
            direction: MetricDrilldownSortDirection::Desc,
        }
    }
}

impl MetricDrilldownSortDirection {
    pub(super) fn as_sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }

    /// INVARIANT: every element of the ordering key travels in this one
    /// direction, so the next page is one tuple comparison rather than a
    /// per-column chain.
    pub(super) fn cursor_operator(self) -> &'static str {
        match self {
            Self::Asc => ">",
            Self::Desc => "<",
        }
    }
}

pub(super) const DATE_KEY: &str = "date";
pub(super) const VALUE_KEY: &str = "value";
pub(super) const NUMERATOR_KEY: &str = "numerator";
pub(super) const DENOMINATOR_KEY: &str = "denominator";
pub(super) const PERSON_KEY: &str = "person";

/// A presented column as the query reads it, applying the same
/// details-then-dimension fallback the row projection does — an order or a
/// search that read the column any other way would answer about a cell the
/// client never rendered.
///
/// The ordering forms take the name the text was bound to rather than the text
/// itself: the flag, the key, the cursor and the ORDER BY all mention it, and
/// the fallback expression is long.
#[derive(Debug)]
pub(super) struct ColumnSql {
    text: String,
    r#type: MetricDrilldownColumnType,
    /// Set where the row already carries the value in a form worth ordering by
    /// directly rather than through its text.
    native_order: Option<String>,
}

impl ColumnSql {
    fn projected(r#type: MetricDrilldownColumnType, text: String) -> Self {
        Self {
            text,
            r#type,
            native_order: None,
        }
    }

    fn native_number(text: String, order: String) -> Self {
        Self {
            text,
            r#type: MetricDrilldownColumnType::Number,
            native_order: Some(order),
        }
    }

    /// The cell as text, empty exactly where the client renders nothing.
    pub(super) fn as_text(&self) -> &str {
        &self.text
    }

    pub(super) fn order_key(&self, source: &str) -> String {
        if let Some(native) = &self.native_order {
            return native.clone();
        }
        match self.r#type {
            MetricDrilldownColumnType::Number => {
                format!("ifNull(toFloat64OrNull(trimBoth({source})), 0)")
            }
            // ISO dates already sort as text, and padding one would only make
            // it stop.
            MetricDrilldownColumnType::Date => format!("trimBoth({source})"),
            MetricDrilldownColumnType::String => natural_order(source),
        }
    }

    /// INVARIANT: a cursor carries its key as text, so a numeric order has to
    /// re-enter the comparison as a number — a tuple cannot compare the two.
    pub(super) fn cursor_binding(&self) -> &'static str {
        match self.r#type {
            MetricDrilldownColumnType::Number => "toFloat64(?)",
            MetricDrilldownColumnType::String | MetricDrilldownColumnType::Date => "?",
        }
    }

    /// Whether a replayed cursor key can re-enter this column's comparison.
    ///
    /// SAFETY: the cast in `cursor_binding` is ClickHouse's, and it refuses the
    /// whole QUERY rather than the value — a cursor whose key does not fit
    /// would be answered with a server error instead of a refusal. A cursor is
    /// caller-held bytes, so this is checked before it is bound.
    pub(super) fn accepts_cursor_key(&self, key: &str) -> bool {
        match self.r#type {
            MetricDrilldownColumnType::Number => key.parse::<f64>().is_ok_and(f64::is_finite),
            MetricDrilldownColumnType::String | MetricDrilldownColumnType::Date => true,
        }
    }
}

/// Blank cells land last whichever way the column is sorted, and this flag is
/// what buys that without a second ORDER BY direction.
pub(super) fn empty_flag(source: &str, direction: MetricDrilldownSortDirection) -> String {
    let comparison = match direction {
        MetricDrilldownSortDirection::Asc => "=",
        MetricDrilldownSortDirection::Desc => "!=",
    };
    format!(
        "toUInt8({blank} {comparison} '')",
        blank = blank_test(source)
    )
}

/// Is this cell blank, by the same rule the row projection applies?
///
/// INVARIANT: ClickHouse `trimBoth` takes spaces alone, while `visible_value`
/// trims by Rust's `str::trim`. Folding the other ASCII whitespace to a space
/// first makes the two agree on the forms a record actually carries — a commit
/// subject of nothing but newlines reads blank on screen, so it has to read
/// blank to the order as well. Whitespace beyond ASCII still parts them.
fn blank_test(source: &str) -> String {
    let mut folded = source.to_owned();
    for whitespace in ["\\t", "\\n", "\\r", "\\v", "\\f"] {
        folded = format!("replaceAll({folded}, '{whitespace}', ' ')");
    }
    format!("trimBoth({folded})")
}

/// Digits sort as digits: `300` before `2958`, the way the reader reads them.
/// Anything else keeps the lexicographic order it already had.
fn natural_order(text: &str) -> String {
    format!(
        "if(toUInt64OrNull(trimBoth({text})) IS NOT NULL \
           AND length(trimBoth({text})) < {NUMERIC_PAD_WIDTH}, \
           leftPad(trimBoth({text}), {NUMERIC_PAD_WIDTH}, '0'), \
           trimBoth({text}))"
    )
}

/// How a presented column is read off the row. `None` is a column the query
/// cannot order or search by, and the validator refuses a sort naming it.
///
/// One resolver for both query shapes: the validator and the compiler ask the
/// same question, and a second implementation is a second answer.
pub(super) fn column_sql(
    key: &str,
    r#type: MetricDrilldownColumnType,
    ratio: bool,
) -> Option<ColumnSql> {
    if ratio {
        return ratio_column_sql(key);
    }
    let sql = match key {
        PERSON_KEY | NUMERATOR_KEY | DENOMINATOR_KEY => return None,
        DATE_KEY => ColumnSql::projected(
            MetricDrilldownColumnType::Date,
            "toString(evidence.metric_date)".to_owned(),
        ),
        VALUE_KEY => ColumnSql::native_number(
            "ifNull(toString(evidence.contribution), '')".to_owned(),
            "ifNull(evidence.contribution, 0)".to_owned(),
        ),
        key => ColumnSql::projected(r#type, detail_text(key)),
    };
    Some(sql)
}

/// A ratio row is two aggregates over a day, so the day and the two numbers are
/// everything it has to be ordered by.
fn ratio_column_sql(key: &str) -> Option<ColumnSql> {
    let sql = match key {
        DATE_KEY => ColumnSql::projected(
            MetricDrilldownColumnType::Date,
            "toString(metric_date)".to_owned(),
        ),
        NUMERATOR_KEY => ColumnSql::native_number(
            "ifNull(toString(numerator), '')".to_owned(),
            "ifNull(numerator, 0)".to_owned(),
        ),
        DENOMINATOR_KEY => ColumnSql::native_number(
            "ifNull(toString(denominator), '')".to_owned(),
            "ifNull(denominator, 0)".to_owned(),
        ),
        _ => return None,
    };
    Some(sql)
}

/// Whether the query can order by a column at all — the one answer the
/// presented column carries and the validator refuses a sort against.
pub(super) fn is_sortable(key: &str, r#type: MetricDrilldownColumnType, ratio: bool) -> bool {
    column_sql(key, r#type, ratio).is_some()
}

/// INVARIANT: mirrors `project_row` — the details map first, the dimension of
/// the same key as the fallback, and a dimension's label ahead of its value.
fn detail_text(key: &str) -> String {
    let literal = sql_string_literal(key);
    format!(
        "if({present} != '', \
           evidence.details[{literal}], \
           {dimension})",
        present = blank_test(&format!("evidence.details[{literal}]")),
        dimension = dimension_text(&literal),
    )
}

fn dimension_text(literal: &str) -> String {
    format!(
        "arrayMap(dimension -> if(trimBoth(ifNull(dimension.3, '')) != '', \
                                  ifNull(dimension.3, ''), \
                                  dimension.2), \
                  arrayFilter(dimension -> dimension.1 = {literal}, evidence.dimensions))[1]"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(key: &str) -> ColumnSql {
        column_sql(key, MetricDrilldownColumnType::String, false)
            .unwrap_or_else(|| panic!("{key} must be readable"))
    }

    #[test]
    fn a_column_the_query_cannot_read_refuses_to_produce_sql() {
        for key in [PERSON_KEY, NUMERATOR_KEY, DENOMINATOR_KEY] {
            assert!(
                column_sql(key, MetricDrilldownColumnType::String, false).is_none(),
                "should refuse: {key}"
            );
        }
        assert!(column_sql("repository", MetricDrilldownColumnType::String, true).is_none());
    }

    #[test]
    fn a_ratio_row_sorts_only_by_the_day_and_the_two_numbers() {
        for key in [DATE_KEY, NUMERATOR_KEY, DENOMINATOR_KEY] {
            assert!(
                is_sortable(key, MetricDrilldownColumnType::Number, true),
                "should sort: {key}"
            );
        }
        assert!(!is_sortable(
            VALUE_KEY,
            MetricDrilldownColumnType::Number,
            true
        ));
    }

    #[test]
    fn a_detail_column_falls_back_to_the_dimension_of_the_same_key() {
        let sql = text("repository");
        assert!(sql.as_text().contains("evidence.details['repository']"));
        assert!(sql.as_text().contains("arrayFilter"));
    }

    #[test]
    fn a_number_column_re_enters_the_cursor_as_a_number() {
        let number = column_sql(VALUE_KEY, MetricDrilldownColumnType::Number, false)
            .unwrap_or_else(|| panic!("value must be readable"));
        assert_eq!(number.cursor_binding(), "toFloat64(?)");
        assert_eq!(text(DATE_KEY).cursor_binding(), "?");
    }

    #[test]
    fn the_emptiness_flag_turns_over_with_the_direction() {
        assert!(empty_flag("cell", MetricDrilldownSortDirection::Asc).contains("= ''"));
        assert!(empty_flag("cell", MetricDrilldownSortDirection::Desc).contains("!= ''"));
    }

    // A forged cursor is caller-held bytes; the cast that replays it is
    // ClickHouse's, and it refuses the query rather than the value.
    #[test]
    fn a_cursor_key_that_cannot_re_enter_the_comparison_is_refused() {
        let number = column_sql(VALUE_KEY, MetricDrilldownColumnType::Number, false)
            .unwrap_or_else(|| panic!("value must be readable"));
        assert!(number.accepts_cursor_key("12.5"));
        assert!(!number.accepts_cursor_key("x"));
        assert!(!number.accepts_cursor_key(""));
        assert!(!number.accepts_cursor_key("inf"));
        assert!(text(DATE_KEY).accepts_cursor_key("anything at all"));
    }

    // A commit subject of nothing but newlines renders blank, so it has to
    // sort with the blanks rather than among the filled.
    #[test]
    fn whitespace_beyond_the_space_still_reads_blank() {
        let sql = empty_flag("cell", MetricDrilldownSortDirection::Asc);
        for whitespace in ["\\t", "\\n", "\\r"] {
            assert!(sql.contains(whitespace), "should fold {whitespace}: {sql}");
        }
    }

    #[test]
    fn a_string_column_of_digits_orders_as_digits() {
        let sql = text("ref");
        assert!(sql.order_key("cell").contains("leftPad"));
        assert!(sql.order_key("cell").contains("toUInt64OrNull"));
    }
}
