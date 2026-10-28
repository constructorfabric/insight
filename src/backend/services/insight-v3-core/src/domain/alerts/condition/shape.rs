//! The condition's JSON shape, for the API document and the MCP tool
//! schemas. A draft is read by [`super::Condition::parse`], which names the
//! path of whatever is wrong; these types only describe what it accepts.
#![expect(
    dead_code,
    reason = "schema descriptions only; drafts are parsed from JSON"
)]

use schemars::JsonSchema;
use utoipa::ToSchema;

/// Which rows must pass, and the test each row takes.
#[derive(Debug, JsonSchema, ToSchema)]
pub(crate) struct ConditionShape {
    /// `one`: the metric must answer exactly one row, and it must pass.
    /// `any`, `all`, `none`: of the rows answered. `{"count": {"op": ">",
    /// "value": 3}}`: the number of passing rows, compared. `any` over no
    /// rows is not met; `all` and `none` over no rows are unknown.
    rows: RowsShape,
    /// A test on one column, or a group of tests. Groups nest at most 3
    /// deep and hold at most 20 tests in all.
    #[serde(rename = "where")]
    test: NodeShape,
}

#[derive(Debug, JsonSchema, ToSchema)]
#[serde(untagged)]
enum RowsShape {
    Word(RowsWord),
    Count { count: CountShape },
}

#[derive(Debug, JsonSchema, ToSchema)]
#[serde(rename_all = "lowercase")]
enum RowsWord {
    One,
    Any,
    All,
    None,
}

#[derive(Debug, JsonSchema, ToSchema)]
struct CountShape {
    /// One of `>`, `>=`, `<`, `<=`, `==`, `!=`.
    op: String,
    value: u64,
}

#[derive(Debug, JsonSchema, ToSchema)]
#[serde(untagged)]
enum NodeShape {
    /// Met when every test in it is.
    All {
        #[schema(no_recursion)]
        all: Vec<NodeShape>,
    },
    /// Met when one test in it is.
    Any {
        #[schema(no_recursion)]
        any: Vec<NodeShape>,
    },
    /// Met when the test in it is not.
    Not {
        #[schema(no_recursion)]
        not: Box<NodeShape>,
    },
    Test(LeafShape),
}

/// One test on one result column. Which operators apply depends on what
/// the column holds: a number takes `>`, `>=`, `<`, `<=`, `==`, `!=`,
/// `between`; text takes `==`, `!=`, `contains`, `starts_with`,
/// `ends_with`, `in`, `not_in`, comparing exactly; a date takes `before`,
/// `after`, `between`, `older_than`, `newer_than`.
#[derive(Debug, JsonSchema, ToSchema)]
struct LeafShape {
    /// The result column, by the `as_name` the metric gives it.
    column: String,
    op: OperatorShape,
    /// A number, or an integer written as digits; text up to 256
    /// characters; an RFC 3339 timestamp; a list of two for `between`; a
    /// list of 1 to 50 texts for `in` and `not_in`; a duration such as
    /// `P7D`, `P2W`, `PT6H` or `PT30M` for `older_than` and `newer_than`,
    /// measured back from the check.
    value: serde_json::Value,
}

#[derive(Debug, JsonSchema, ToSchema)]
enum OperatorShape {
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = "==")]
    Eq,
    #[serde(rename = "!=")]
    Ne,
    #[serde(rename = "between")]
    Between,
    #[serde(rename = "contains")]
    Contains,
    #[serde(rename = "starts_with")]
    StartsWith,
    #[serde(rename = "ends_with")]
    EndsWith,
    #[serde(rename = "in")]
    In,
    #[serde(rename = "not_in")]
    NotIn,
    #[serde(rename = "before")]
    Before,
    #[serde(rename = "after")]
    After,
    #[serde(rename = "older_than")]
    OlderThan,
    #[serde(rename = "newer_than")]
    NewerThan,
}
