//! The wire shape of a query and of the answer it gets.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const MAX_FILTERS: usize = 32;
pub const MAX_FILTER_VALUES: usize = 256;
pub const MAX_AGGREGATES: usize = 16;
pub const MAX_GROUP_AXES: usize = 4;
pub const MAX_ORDER_TERMS: usize = 4;
pub const MAX_NAME_CHARS: usize = 64;
pub const DEFAULT_ROW_LIMIT: u32 = 1_000;
pub const MAX_ROW_LIMIT: u32 = 10_000;

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = Query)]
pub struct QueryRequest {
    pub dataset: String,
    /// Row filters, narrowing the scan before aggregation.
    #[serde(default)]
    pub filters: Vec<FilterDto>,
    #[serde(default)]
    pub group_by: Vec<GroupAxisDto>,
    pub aggregates: Vec<AggregateDto>,
    /// The window every scan is bounded by, and the width of its buckets.
    pub time: TimeDto,
    #[serde(default)]
    pub order: Vec<OrderDto>,
    /// Row ceiling; a query over the cap is refused rather than clipped.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
#[schema(as = QueryFilter)]
pub enum FilterDto {
    Eq(EqFilterDto),
    In(InFilterDto),
    Gt(CompareFilterDto),
    Gte(CompareFilterDto),
    Lt(CompareFilterDto),
    Lte(CompareFilterDto),
    Between(BetweenFilterDto),
    NotNull(NotNullFilterDto),
}

impl FilterDto {
    pub fn field(&self) -> &str {
        match self {
            Self::Eq(filter) => &filter.field,
            Self::In(filter) => &filter.field,
            Self::Gt(filter) | Self::Gte(filter) | Self::Lt(filter) | Self::Lte(filter) => {
                &filter.field
            }
            Self::Between(filter) => &filter.field,
            Self::NotNull(filter) => &filter.field,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = QueryFilterEq)]
pub struct EqFilterDto {
    /// A declared dimension or measurable of the dataset.
    pub field: String,
    pub value: ScalarDto,
}

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = QueryFilterIn)]
pub struct InFilterDto {
    /// A declared dimension or measurable of the dataset.
    pub field: String,
    /// At least one value, and at most the contract's cap.
    pub values: Vec<ScalarDto>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = QueryFilterCompare)]
pub struct CompareFilterDto {
    /// A declared dimension or measurable of the dataset.
    pub field: String,
    pub value: ScalarDto,
}

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = QueryFilterBetween)]
pub struct BetweenFilterDto {
    /// A declared dimension or measurable of the dataset.
    pub field: String,
    /// Inclusive lower bound.
    pub low: ScalarDto,
    /// Inclusive upper bound.
    pub high: ScalarDto,
}

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = QueryFilterNotNull)]
pub struct NotNullFilterDto {
    /// A declared dimension or measurable of the dataset.
    pub field: String,
}

/// A filter value, in the JSON type it was written as.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ScalarDto {
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
}

// The derive cannot annotate a newtype variant's field, and `serde_json::Number`
// has no schema of its own; the contract is simply "one JSON scalar".
impl utoipa::PartialSchema for ScalarDto {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::OneOfBuilder::new()
            .item(<String as utoipa::PartialSchema>::schema())
            .item(<f64 as utoipa::PartialSchema>::schema())
            .item(<bool as utoipa::PartialSchema>::schema())
            .description(Some("A filter value: text, a number, or a boolean"))
            .into()
    }
}

impl utoipa::ToSchema for ScalarDto {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("QueryFilterValue")
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(tag = "axis", rename_all = "snake_case")]
#[schema(as = QueryGroupAxis)]
pub enum GroupAxisDto {
    Dimension(DimensionAxisDto),
    Time(TimeAxisDto),
}

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = QueryDimensionAxis)]
pub struct DimensionAxisDto {
    /// A declared dimension of the dataset.
    pub field: String,
}

/// The time bucket as a group axis; its width is `time.grain`.
#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = QueryTimeAxis)]
pub struct TimeAxisDto {}

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(tag = "fn", rename_all = "snake_case")]
#[schema(as = QueryAggregate)]
pub enum AggregateDto {
    Count(CountAggregateDto),
    Sum(FoldAggregateDto),
    Avg(FoldAggregateDto),
    Min(FoldAggregateDto),
    Max(FoldAggregateDto),
}

impl AggregateDto {
    pub fn name(&self) -> &str {
        match self {
            Self::Count(aggregate) => &aggregate.name,
            Self::Sum(aggregate)
            | Self::Avg(aggregate)
            | Self::Min(aggregate)
            | Self::Max(aggregate) => &aggregate.name,
        }
    }

    pub fn filter(&self) -> Option<&FilterDto> {
        match self {
            Self::Count(aggregate) => aggregate.filter.as_ref(),
            Self::Sum(aggregate)
            | Self::Avg(aggregate)
            | Self::Min(aggregate)
            | Self::Max(aggregate) => aggregate.filter.as_ref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = QueryAggregateCount)]
pub struct CountAggregateDto {
    /// What the answer column is called.
    pub name: String,
    /// Restricts this fold to the rows one filter selects.
    #[serde(default)]
    pub filter: Option<FilterDto>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = QueryAggregateFold)]
pub struct FoldAggregateDto {
    /// What the answer column is called.
    pub name: String,
    /// A declared measurable of the dataset.
    pub field: String,
    /// Restricts this fold to the rows one filter selects.
    #[serde(default)]
    pub filter: Option<FilterDto>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = QueryTime)]
pub struct TimeDto {
    /// A declared time field; the dataset's default when omitted.
    #[serde(default)]
    pub field: Option<String>,
    /// Inclusive first day, UTC.
    pub from: NaiveDate,
    /// Inclusive last day, UTC.
    pub to: NaiveDate,
    /// Bucket width. Required with an `{"axis": "time"}` axis, refused without one.
    #[serde(default)]
    pub grain: Option<Grain>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = QueryGrain)]
pub enum Grain {
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = QueryOrder)]
pub struct OrderDto {
    /// A column the answer reports: a grouped dimension, `time`, or an aggregate name.
    pub by: String,
    #[serde(default)]
    pub dir: Direction,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = QueryDirection)]
pub enum Direction {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QueryAnswer {
    pub columns: Vec<AnswerColumn>,
    /// One cell per column, in column order; a fold that observed nothing is null.
    #[schema(schema_with = answer_rows_schema)]
    pub rows: Vec<Vec<serde_json::Value>>,
}

fn answer_rows_schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
    use utoipa::openapi::schema::{ArrayBuilder, Object, SchemaType};

    let cell = Object::with_type(SchemaType::AnyValue);
    ArrayBuilder::new()
        .items(ArrayBuilder::new().items(cell))
        .into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = QueryAnswerColumn)]
pub struct AnswerColumn {
    pub name: String,
    pub kind: ColumnKind,
    #[serde(rename = "type")]
    pub value_type: ColumnType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = QueryColumnKind)]
pub enum ColumnKind {
    Dimension,
    Bucket,
    Aggregate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = QueryColumnType)]
pub enum ColumnType {
    Text,
    Number,
    Date,
}

impl toolkit::api::api_dto::RequestApiDto for QueryRequest {}
impl toolkit::api::api_dto::ResponseApiDto for QueryAnswer {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<QueryRequest, serde_json::Error> {
        serde_json::from_str(json)
    }

    const MINIMAL: &str = r#"{
      "dataset": "git_commits",
      "aggregates": [{"name": "commits", "fn": "count"}],
      "time": {"from": "2026-01-01", "to": "2026-01-31"}
    }"#;

    #[test]
    fn a_query_naming_only_what_it_needs_deserializes() {
        let query = parse(MINIMAL).expect("the minimal query is in the contract");

        assert_eq!(query.dataset, "git_commits");
        assert!(query.filters.is_empty());
        assert!(query.group_by.is_empty());
        assert!(query.order.is_empty());
        assert_eq!(query.limit, None);
        assert_eq!(query.time.grain, None);
        assert!(matches!(query.aggregates[0], AggregateDto::Count(_)));
    }

    #[test]
    fn a_key_the_contract_does_not_declare_is_refused_at_deserialization() {
        let cases = [
            r#"{"dataset": "d", "aggregates": [], "time": {"from": "2026-01-01", "to": "2026-01-02"}, "having": []}"#,
            r#"{"dataset": "d", "aggregates": [{"name": "n", "fn": "count", "quantile": 0.9}], "time": {"from": "2026-01-01", "to": "2026-01-02"}}"#,
            r#"{"dataset": "d", "aggregates": [], "time": {"from": "2026-01-01", "to": "2026-01-02", "timezone": "UTC"}}"#,
            r#"{"dataset": "d", "aggregates": [], "filters": [{"field": "f", "op": "eq", "values": [1], "extra": 2}], "time": {"from": "2026-01-01", "to": "2026-01-02"}}"#,
            r#"{"dataset": "d", "aggregates": [], "order": [{"by": "n", "dir": "asc", "nulls": "last"}], "time": {"from": "2026-01-01", "to": "2026-01-02"}}"#,
        ];
        for json in cases {
            assert!(parse(json).is_err(), "{json}");
        }
    }

    #[test]
    fn an_enumerated_field_refuses_a_value_outside_its_enumeration() {
        let cases = [
            r#"{"dataset": "d", "aggregates": [{"name": "n", "fn": "median"}], "time": {"from": "2026-01-01", "to": "2026-01-02"}}"#,
            r#"{"dataset": "d", "aggregates": [], "filters": [{"field": "f", "op": "like", "value": "x"}], "time": {"from": "2026-01-01", "to": "2026-01-02"}}"#,
            r#"{"dataset": "d", "aggregates": [], "time": {"from": "2026-01-01", "to": "2026-01-02", "grain": "quarter"}}"#,
            r#"{"dataset": "d", "aggregates": [], "order": [{"by": "n", "dir": "sideways"}], "time": {"from": "2026-01-01", "to": "2026-01-02"}}"#,
        ];
        for json in cases {
            assert!(parse(json).is_err(), "{json}");
        }
    }

    #[test]
    fn a_group_axis_is_either_a_dimension_or_the_time_bucket() {
        let query = parse(
            r#"{
              "dataset": "git_commits",
              "group_by": [{"axis": "dimension", "field": "repository"}, {"axis": "time"}],
              "aggregates": [{"name": "commits", "fn": "count"}],
              "time": {"from": "2026-01-01", "to": "2026-01-31", "grain": "week"}
            }"#,
        )
        .expect("both axis shapes are in the contract");

        assert!(matches!(query.group_by[0], GroupAxisDto::Dimension(_)));
        assert!(matches!(query.group_by[1], GroupAxisDto::Time(_)));

        assert!(
            parse(
                r#"{"dataset": "d", "group_by": [{"bin": {"field": "f", "width": 10}}],
                    "aggregates": [], "time": {"from": "2026-01-01", "to": "2026-01-02"}}"#
            )
            .is_err(),
            "a bin axis is not a shape this contract carries yet"
        );
    }

    #[test]
    fn a_filter_value_keeps_the_json_type_it_was_written_as() {
        let query = parse(
            r#"{
              "dataset": "git_commits",
              "filters": [
                {"field": "source", "op": "eq", "value": "github"},
                {"field": "lines_added", "op": "gte", "value": 500},
                {"field": "lines_added", "op": "not_null"}
              ],
              "aggregates": [{"name": "commits", "fn": "count"}],
              "time": {"from": "2026-01-01", "to": "2026-01-31"}
            }"#,
        )
        .expect("the filter shapes are in the contract");

        assert_eq!(
            query.filters[0],
            FilterDto::Eq(EqFilterDto {
                field: "source".to_owned(),
                value: ScalarDto::Text("github".to_owned()),
            })
        );
        assert!(matches!(
            &query.filters[1],
            FilterDto::Gte(compare) if matches!(compare.value, ScalarDto::Number(_))
        ));
        assert!(matches!(query.filters[2], FilterDto::NotNull(_)));
    }

    fn filter(json: &str) -> Result<FilterDto, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn every_filter_operator_carries_exactly_the_operands_it_takes() {
        let cases = [
            (
                r#"{"op": "eq", "field": "source", "value": "github"}"#,
                FilterDto::Eq(EqFilterDto {
                    field: "source".to_owned(),
                    value: ScalarDto::Text("github".to_owned()),
                }),
            ),
            (
                r#"{"op": "in", "field": "source", "values": ["github", "gitlab"]}"#,
                FilterDto::In(InFilterDto {
                    field: "source".to_owned(),
                    values: vec![
                        ScalarDto::Text("github".to_owned()),
                        ScalarDto::Text("gitlab".to_owned()),
                    ],
                }),
            ),
            (
                r#"{"op": "lt", "field": "lines_added", "value": 10}"#,
                FilterDto::Lt(CompareFilterDto {
                    field: "lines_added".to_owned(),
                    value: ScalarDto::Number(10.into()),
                }),
            ),
            (
                r#"{"op": "between", "field": "lines_added", "low": 1, "high": 9}"#,
                FilterDto::Between(BetweenFilterDto {
                    field: "lines_added".to_owned(),
                    low: ScalarDto::Number(1.into()),
                    high: ScalarDto::Number(9.into()),
                }),
            ),
            (
                r#"{"op": "not_null", "field": "source_id"}"#,
                FilterDto::NotNull(NotNullFilterDto {
                    field: "source_id".to_owned(),
                }),
            ),
        ];

        for (json, expected) in cases {
            assert_eq!(filter(json).expect(json), expected, "{json}");
        }
    }

    #[test]
    fn an_operand_belonging_to_another_operator_is_refused_at_deserialization() {
        let cases = [
            r#"{"op": "eq", "field": "source", "values": ["github"]}"#,
            r#"{"op": "eq", "field": "source", "value": "a", "high": "b"}"#,
            r#"{"op": "in", "field": "source", "value": "github"}"#,
            r#"{"op": "gte", "field": "lines_added", "low": 1, "high": 9}"#,
            r#"{"op": "between", "field": "lines_added", "value": 1}"#,
            r#"{"op": "between", "field": "lines_added", "low": 1}"#,
            r#"{"op": "not_null", "field": "source_id", "value": 1}"#,
            r#"{"field": "source", "value": "github"}"#,
        ];

        for json in cases {
            assert!(filter(json).is_err(), "{json}");
        }
    }

    fn aggregate(json: &str) -> Result<AggregateDto, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn every_fold_carries_a_column_exactly_when_it_reads_one() {
        let count = aggregate(r#"{"fn": "count", "name": "commits"}"#).expect("a count");
        assert_eq!(
            count,
            AggregateDto::Count(CountAggregateDto {
                name: "commits".to_owned(),
                filter: None,
            })
        );

        for function in ["sum", "avg", "min", "max"] {
            let json = format!(r#"{{"fn": "{function}", "name": "v", "field": "lines_added"}}"#);
            let parsed = aggregate(&json).unwrap_or_else(|error| panic!("{json}: {error}"));
            assert_eq!(parsed.name(), "v", "{json}");
        }
    }

    #[test]
    fn a_fold_that_names_the_wrong_operands_is_refused_at_deserialization() {
        let cases = [
            r#"{"fn": "count", "name": "commits", "field": "lines_added"}"#,
            r#"{"fn": "sum", "name": "added"}"#,
            r#"{"fn": "median", "name": "v", "field": "lines_added"}"#,
            r#"{"name": "v", "field": "lines_added"}"#,
        ];

        for json in cases {
            assert!(aggregate(json).is_err(), "{json}");
        }
    }

    #[test]
    fn a_group_axis_names_the_axis_it_is() {
        let cases = [
            r#"{"axis": "dimension", "field": "repository"}"#,
            r#"{"axis": "time"}"#,
        ];
        for json in cases {
            serde_json::from_str::<GroupAxisDto>(json).unwrap_or_else(|error| {
                panic!("{json}: {error}");
            });
        }

        let refused = [
            r#"{"axis": "time", "dimension": "repository"}"#,
            r#"{"axis": "dimension"}"#,
            r#"{"dimension": "repository"}"#,
            r#"{"axis": "bin", "field": "lines_added", "width": 10}"#,
        ];
        for json in refused {
            assert!(
                serde_json::from_str::<GroupAxisDto>(json).is_err(),
                "{json}"
            );
        }
    }
}
