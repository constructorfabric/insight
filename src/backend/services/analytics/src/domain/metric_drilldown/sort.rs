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
}

/// Blank cells land last whichever way the column is sorted, and this flag is
/// what buys that without a second ORDER BY direction.
pub(super) fn empty_flag(source: &str, direction: MetricDrilldownSortDirection) -> String {
    let comparison = match direction {
        MetricDrilldownSortDirection::Asc => "=",
        MetricDrilldownSortDirection::Desc => "!=",
    };
    format!("toUInt8(trimBoth({source}) {comparison} '')")
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

/// How a column of the non-ratio query is read off the evidence row. `None` is
/// a column the query cannot order or search by, and the validator refuses a
/// sort naming it.
pub(super) fn value_column_sql(key: &str, r#type: MetricDrilldownColumnType) -> Option<ColumnSql> {
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
pub(super) fn ratio_column_sql(key: &str, r#type: MetricDrilldownColumnType) -> Option<ColumnSql> {
    let _ = r#type;
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
    if ratio {
        ratio_column_sql(key, r#type).is_some()
    } else {
        value_column_sql(key, r#type).is_some()
    }
}

/// INVARIANT: mirrors `project_row` — the details map first, the dimension of
/// the same key as the fallback, and a dimension's label ahead of its value.
fn detail_text(key: &str) -> String {
    let literal = sql_string_literal(key);
    format!(
        "if(trimBoth(evidence.details[{literal}]) != '', \
           evidence.details[{literal}], \
           {dimension})",
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
        value_column_sql(key, MetricDrilldownColumnType::String)
            .unwrap_or_else(|| panic!("{key} must be readable"))
    }

    #[test]
    fn a_column_the_query_cannot_read_refuses_to_produce_sql() {
        for key in [PERSON_KEY, NUMERATOR_KEY, DENOMINATOR_KEY] {
            assert!(
                value_column_sql(key, MetricDrilldownColumnType::String).is_none(),
                "should refuse: {key}"
            );
        }
        assert!(ratio_column_sql("repository", MetricDrilldownColumnType::String).is_none());
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
        let number = value_column_sql(VALUE_KEY, MetricDrilldownColumnType::Number)
            .unwrap_or_else(|| panic!("value must be readable"));
        assert_eq!(number.cursor_binding(), "toFloat64(?)");
        assert_eq!(text(DATE_KEY).cursor_binding(), "?");
    }

    #[test]
    fn the_emptiness_flag_turns_over_with_the_direction() {
        assert!(empty_flag("cell", MetricDrilldownSortDirection::Asc).contains("= ''"));
        assert!(empty_flag("cell", MetricDrilldownSortDirection::Desc).contains("!= ''"));
    }

    #[test]
    fn a_string_column_of_digits_orders_as_digits() {
        let sql = text("ref");
        assert!(sql.order_key("cell").contains("leftPad"));
        assert!(sql.order_key("cell").contains("toUInt64OrNull"));
    }
}
