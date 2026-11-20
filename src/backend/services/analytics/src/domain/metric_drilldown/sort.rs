use serde::{Deserialize, Serialize};

use crate::domain::metric_results::compiler::sql_string_literal;

use super::dto::MetricDrilldownColumnType;
use super::row_shape::RowShape;

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
pub(super) const RECORDS_THAT_DAY_KEY: &str = "records_that_day";
pub(super) const PERSON_KEY: &str = "person";
pub(super) const PARTS_KEY: &str = "parts";

#[derive(Debug)]
pub(super) struct ColumnSql {
    text: String,
    r#type: MetricDrilldownColumnType,
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
            MetricDrilldownColumnType::Date => format!("trimBoth({source})"),
            MetricDrilldownColumnType::String => natural_order(source),
        }
    }

    pub(super) fn cursor_binding(&self) -> &'static str {
        match self.r#type {
            MetricDrilldownColumnType::Number => "toFloat64(?)",
            MetricDrilldownColumnType::String | MetricDrilldownColumnType::Date => "?",
        }
    }

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

fn natural_order(text: &str) -> String {
    format!(
        "if(toUInt64OrNull(trimBoth({text})) IS NOT NULL \
           AND length(trimBoth({text})) < {NUMERIC_PAD_WIDTH}, \
           leftPad(trimBoth({text}), {NUMERIC_PAD_WIDTH}, '0'), \
           trimBoth({text}))"
    )
}

pub(super) fn column_sql(
    key: &str,
    r#type: MetricDrilldownColumnType,
    shape: RowShape,
) -> Option<ColumnSql> {
    match shape {
        RowShape::Record | RowShape::FoldedRecord => record_column_sql(key, r#type),
        RowShape::DailyRatio => ratio_column_sql(key),
        RowShape::RecordRatio => record_ratio_column_sql(key, r#type),
        RowShape::RecordsPerSubject => records_per_subject_column_sql(key, r#type),
    }
}

fn record_column_sql(key: &str, r#type: MetricDrilldownColumnType) -> Option<ColumnSql> {
    let sql = match key {
        PERSON_KEY | NUMERATOR_KEY | DENOMINATOR_KEY | RECORDS_THAT_DAY_KEY => return None,
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

fn record_ratio_column_sql(key: &str, r#type: MetricDrilldownColumnType) -> Option<ColumnSql> {
    match key {
        VALUE_KEY => None,
        NUMERATOR_KEY | DENOMINATOR_KEY => ratio_column_sql(key),
        key => record_column_sql(key, r#type),
    }
}

fn records_per_subject_column_sql(
    key: &str,
    r#type: MetricDrilldownColumnType,
) -> Option<ColumnSql> {
    match key {
        VALUE_KEY | NUMERATOR_KEY => None,
        RECORDS_THAT_DAY_KEY => ratio_column_sql(DENOMINATOR_KEY),
        key => record_column_sql(key, r#type),
    }
}

pub(super) fn is_sortable(key: &str, r#type: MetricDrilldownColumnType, shape: RowShape) -> bool {
    column_sql(key, r#type, shape).is_some()
}

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

pub(super) fn dimension_text(literal: &str) -> String {
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
        column_sql(key, MetricDrilldownColumnType::String, RowShape::Record)
            .unwrap_or_else(|| panic!("{key} must be readable"))
    }

    #[test]
    fn a_column_the_query_cannot_read_refuses_to_produce_sql() {
        for key in [PERSON_KEY, NUMERATOR_KEY, DENOMINATOR_KEY] {
            assert!(
                column_sql(key, MetricDrilldownColumnType::String, RowShape::Record).is_none(),
                "should refuse: {key}"
            );
        }
        assert!(
            column_sql(
                "repository",
                MetricDrilldownColumnType::String,
                RowShape::DailyRatio
            )
            .is_none()
        );
    }

    #[test]
    fn a_ratio_row_sorts_only_by_the_day_and_the_two_numbers() {
        for key in [DATE_KEY, NUMERATOR_KEY, DENOMINATOR_KEY] {
            assert!(
                is_sortable(key, MetricDrilldownColumnType::Number, RowShape::DailyRatio),
                "should sort: {key}"
            );
        }
        assert!(!is_sortable(
            VALUE_KEY,
            MetricDrilldownColumnType::Number,
            RowShape::DailyRatio
        ));
    }

    #[test]
    fn a_ratio_of_records_reads_the_record_and_its_two_numbers() {
        for key in [DATE_KEY, NUMERATOR_KEY, DENOMINATOR_KEY, "ref", "title"] {
            assert!(
                is_sortable(
                    key,
                    MetricDrilldownColumnType::String,
                    RowShape::RecordRatio
                ),
                "should sort: {key}"
            );
        }
        for key in [VALUE_KEY, PERSON_KEY] {
            assert!(
                !is_sortable(
                    key,
                    MetricDrilldownColumnType::Number,
                    RowShape::RecordRatio
                ),
                "should refuse: {key}"
            );
        }
        let title = column_sql(
            "title",
            MetricDrilldownColumnType::String,
            RowShape::RecordRatio,
        )
        .unwrap_or_else(|| panic!("title must be readable"));
        assert!(title.as_text().contains("evidence.details['title']"));
    }

    #[test]
    fn a_detail_column_falls_back_to_the_dimension_of_the_same_key() {
        let sql = text("repository");
        assert!(sql.as_text().contains("evidence.details['repository']"));
        assert!(sql.as_text().contains("arrayFilter"));
    }

    #[test]
    fn a_number_column_re_enters_the_cursor_as_a_number() {
        let number = column_sql(
            VALUE_KEY,
            MetricDrilldownColumnType::Number,
            RowShape::Record,
        )
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
    fn a_cursor_key_that_cannot_re_enter_the_comparison_is_refused() {
        let number = column_sql(
            VALUE_KEY,
            MetricDrilldownColumnType::Number,
            RowShape::Record,
        )
        .unwrap_or_else(|| panic!("value must be readable"));
        assert!(number.accepts_cursor_key("12.5"));
        assert!(!number.accepts_cursor_key("x"));
        assert!(!number.accepts_cursor_key(""));
        assert!(!number.accepts_cursor_key("inf"));
        assert!(text(DATE_KEY).accepts_cursor_key("anything at all"));
    }

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
