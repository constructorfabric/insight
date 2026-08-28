//! The wire shape of a question about the rows behind a value, and the page
//! that answers it.
//!
//! INVARIANT: a column is described once and every row is an array of values in
//! that column's order, so a page carries no repeated key per row.

use serde::{Deserialize, Serialize};

use super::super::dto::{DimensionFilter, Provenance, Subjects};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RowsRequest {
    /// The metric key the semantic definitions carry, such as `git.commits`.
    pub metric: String,
    pub subjects: Subjects,
    pub time: TimeRange,
    /// Narrows the scan exactly as it narrows the value. Absent means none.
    #[serde(default)]
    pub filters: Vec<DimensionFilter>,
    /// Which input of the metric's computation to page. A metric composing one
    /// input needs none; one composing several names which.
    #[serde(default)]
    pub input: Option<String>,
    /// Dimension keys to report beyond the ones the metric's measure declares.
    #[serde(default)]
    pub display_dimensions: Vec<String>,
    /// Rows per page. Absent means 100.
    #[serde(default)]
    pub page_size: Option<u32>,
    /// Where to resume, as the previous page reported it. Absent asks for the
    /// first page.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeRange {
    /// Inclusive first day, `YYYY-MM-DD`.
    pub from: String,
    /// Inclusive last day, `YYYY-MM-DD`.
    pub to: String,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct RowsResponse {
    pub metric: String,
    pub provenance: Provenance,
    /// The part of the metric's computation these rows were folded into.
    pub input: String,
    pub columns: Vec<RowColumn>,
    /// One entry per row, holding one value per column, in the columns' order.
    pub rows: Vec<Vec<serde_json::Value>>,
    /// Absent when this page is the last one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct RowColumn {
    pub key: String,
    pub kind: ColumnKind,
    /// The column's name as a reader sees it, derived from its key.
    pub label: String,
}

/// How a column's values read, so a caller renders a page without matching key
/// spellings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKind {
    Text,
    Number,
    /// A calendar day, `YYYY-MM-DD`.
    Date,
    /// A point in time, as the dataset recorded it.
    Timestamp,
}

impl toolkit::api::api_dto::RequestApiDto for RowsRequest {}
impl toolkit::api::api_dto::ResponseApiDto for RowsResponse {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::super::dto::{Executor, ServedFrom};
    use super::*;

    #[test]
    fn every_way_of_asking_for_rows_parses_from_the_shape_it_is_documented_as() {
        let cases = [
            serde_json::json!({
                "metric": "git.commits",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
            }),
            serde_json::json!({
                "metric": "git.merge_rate",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
                "input": "numerator",
                "filters": [{ "dimension": "repository", "values": ["acme/app"] }],
                "display_dimensions": ["repository"],
                "page_size": 25,
                "cursor": "eyJ2IjoxfQ",
            }),
        ];

        for query in cases {
            let named = query.to_string();

            let parsed: Result<RowsRequest, _> = serde_json::from_value(query);

            assert!(parsed.is_ok(), "should parse: {named}");
        }
    }

    #[test]
    fn a_field_the_contract_does_not_declare_is_refused_rather_than_ignored() {
        let cases = [
            serde_json::json!({
                "metric": "git.commits",
                "subjects": { "type": "persons", "ids": [] },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
                "grain": "day",
            }),
            serde_json::json!({
                "metric": "git.commits",
                "subjects": { "type": "persons", "ids": [] },
                "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": "total" },
            }),
            serde_json::json!({
                "metric": "git.commits",
                "subjects": { "type": "persons", "ids": [] },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
                "sort": "date",
            }),
        ];

        for query in cases {
            let named = query.to_string();

            let parsed: Result<RowsRequest, _> = serde_json::from_value(query);

            assert!(parsed.is_err(), "should refuse: {named}");
        }
    }

    #[test]
    fn a_page_is_serialized_as_the_documented_shape() {
        let response = RowsResponse {
            metric: "git.commits".to_owned(),
            provenance: Provenance {
                executor: Executor::Semantic,
                definition_version: Some(4),
                served_from: ServedFrom::Computed,
            },
            input: "value".to_owned(),
            columns: vec![
                RowColumn {
                    key: "subject".to_owned(),
                    kind: ColumnKind::Text,
                    label: "Subject".to_owned(),
                },
                RowColumn {
                    key: "date".to_owned(),
                    kind: ColumnKind::Date,
                    label: "Date".to_owned(),
                },
            ],
            rows: vec![vec![
                serde_json::Value::from("00000000-0000-0000-0000-000000000001"),
                serde_json::Value::from("2026-01-05"),
            ]],
            next_cursor: Some("eyJ2IjoxfQ".to_owned()),
        };

        assert_eq!(
            serde_json::to_value(&response).expect("the page serializes"),
            serde_json::json!({
                "metric": "git.commits",
                "provenance": { "served_from": "computed", "executor": "semantic", "definition_version": 4 },
                "input": "value",
                "columns": [
                    { "key": "subject", "kind": "text", "label": "Subject" },
                    { "key": "date", "kind": "date", "label": "Date" },
                ],
                "rows": [["00000000-0000-0000-0000-000000000001", "2026-01-05"]],
                "next_cursor": "eyJ2IjoxfQ",
            })
        );
    }

    #[test]
    fn the_last_page_reports_no_position_to_resume_from() {
        let response = RowsResponse {
            metric: "git.commits".to_owned(),
            provenance: Provenance {
                executor: Executor::Semantic,
                definition_version: None,
                served_from: ServedFrom::Computed,
            },
            input: "value".to_owned(),
            columns: Vec::new(),
            rows: Vec::new(),
            next_cursor: None,
        };

        assert_eq!(
            serde_json::to_value(&response).expect("the page serializes"),
            serde_json::json!({
                "metric": "git.commits",
                "provenance": { "served_from": "computed", "executor": "semantic" },
                "input": "value",
                "columns": [],
                "rows": [],
            })
        );
    }
}
