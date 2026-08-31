//! The wire shape of a values question and its answer.
//!
//! INVARIANT: nothing here names a view, a relation or a row shape, and the
//! answer carries no sentinel — a total is a field, a leftover is a variant.

use serde::{Deserialize, Serialize};

use super::super::dto::{DimensionFilter, Provenance, Subjects};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ValuesRequest {
    pub queries: Vec<ValuesQuery>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ValuesQuery {
    /// The metric key the semantic definitions carry, such as `git.commits`.
    pub metric: String,
    pub subjects: Subjects,
    pub time: TimeRange,
    /// Narrows every measure the metric reads. Absent means no narrowing.
    #[serde(default)]
    pub filters: Vec<DimensionFilter>,
    #[serde(default)]
    pub split: Option<Split>,
    pub fold: Fold,
    #[serde(default)]
    pub compare: Option<Compare>,
}

/// Asks the same question again over an earlier window, and reports the change.
#[derive(Debug, Clone, Copy, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Compare {
    pub offset: CompareOffset,
}

/// How far back the compared window sits: `previous_period` shifts it by its
/// own length, and a calendar offset shifts its first day back by that many
/// calendar months, clamping a day the earlier month does not have to that
/// month's last day. Either way the compared window spans as many days as the
/// one it is compared against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompareOffset {
    PreviousPeriod,
    Month,
    Quarter,
    Year,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeRange {
    /// Inclusive first day, `YYYY-MM-DD`.
    pub from: String,
    /// Inclusive last day, `YYYY-MM-DD`.
    pub to: String,
    pub grain: Grain,
}

/// How finely the window is cut. `total` folds it whole; the rest report a
/// point per bucket the metric observed an event in, beside the window total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Grain {
    Total,
    Day,
    Week,
    Month,
}

/// Which dimensions the value is broken out by, and how many of their groups
/// the answer keeps.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Split {
    pub dimensions: Vec<String>,
    #[serde(default)]
    pub limit: Option<SplitLimit>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SplitLimit {
    /// How many groups to keep.
    pub top: u32,
    /// The metric the groups are ranked by. Defaults to the metric being read.
    #[serde(default)]
    pub rank_by: Option<String>,
    /// Whether everything outside the kept groups is one group, or dropped.
    pub remainder: bool,
}

/// Whether each subject keeps its own value or the subjects fold into one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fold {
    PerSubject,
    Combined,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ValuesResponse {
    /// One entry per requested query, in the order they were asked.
    pub results: Vec<QueryResult>,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct QueryResult {
    pub metric: String,
    pub provenance: Provenance,
    pub result: ResultBody,
}

/// The answer's shape, decided by the question's grain: `total` answers with
/// values, every other grain answers with series.
#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum ResultBody {
    Values { values: Vec<GroupedValue> },
    Series { series: Vec<GroupedSeries> },
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct GroupedValue {
    /// Absent when the subjects folded into one value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Absent when the question named no split.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Group>,
    pub value: Option<f64>,
    /// Absent when the question asked for no comparison.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare: Option<Comparison>,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct GroupedSeries {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Group>,
    pub points: Vec<Point>,
    /// The whole window folded once, not the sum of the points.
    pub total: Option<f64>,
    /// The window total beside the compared window's. A series is compared at
    /// the total only; the points carry no comparison of their own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare: Option<Comparison>,
}

/// The compared window's value and the two ways of reading the change.
#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct Comparison {
    /// What the same question answered over the compared window.
    pub value: Option<f64>,
    /// Current minus compared; absent when either side is unknown.
    pub delta: Option<f64>,
    /// Current over compared; absent when the compared value is unknown or
    /// zero, which no ratio is defined against.
    pub ratio: Option<f64>,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct Point {
    /// The bucket's first day, `YYYY-MM-DD`.
    pub date: String,
    pub value: Option<f64>,
}

/// Which slice of the split a row belongs to.
#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Group {
    Dimensions {
        dimensions: Vec<GroupDimension>,
    },
    /// Everything outside the groups a cap kept, folded into one.
    Remainder {},
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct GroupDimension {
    pub key: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl toolkit::api::api_dto::RequestApiDto for ValuesRequest {}
impl toolkit::api::api_dto::ResponseApiDto for ValuesResponse {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::super::dto::{Executor, ServedFrom};
    use super::*;

    #[test]
    fn every_way_of_asking_parses_from_the_shape_it_is_documented_as() {
        let cases = [
            serde_json::json!({
                "metric": "git.commits",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": "total" },
                "fold": "per_subject",
            }),
            serde_json::json!({
                "metric": "git.commits",
                "subjects": { "type": "tenant" },
                "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": "total" },
                "split": { "dimensions": ["repository"] },
                "fold": "combined",
            }),
            serde_json::json!({
                "metric": "git.commits",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-03-31", "grain": "week" },
                "split": {
                    "dimensions": ["repository"],
                    "limit": { "top": 3, "rank_by": "git.commits", "remainder": true },
                },
                "fold": "per_subject",
            }),
            serde_json::json!({
                "metric": "git.commits",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": "month" },
                "fold": "per_subject",
            }),
        ];

        for query in cases {
            let named = query.to_string();
            let body = serde_json::json!({ "queries": [query] });

            let parsed: Result<ValuesRequest, _> = serde_json::from_value(body);

            assert!(parsed.is_ok(), "should parse: {named}");
        }
    }

    #[test]
    fn a_narrowed_and_compared_question_parses_from_the_shape_it_is_documented_as() {
        let body = serde_json::json!({
            "queries": [{
                "metric": "git.commits",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": "total" },
                "filters": [
                    { "dimension": "repository", "values": ["acme/app", "acme/api"] },
                ],
                "compare": { "offset": "previous_period" },
                "fold": "per_subject",
            }],
        });

        let parsed: ValuesRequest = serde_json::from_value(body).expect("the wire shape parses");

        let query = &parsed.queries[0];
        assert_eq!(query.filters[0].dimension, "repository");
        assert_eq!(query.filters[0].values, ["acme/app", "acme/api"]);
        assert_eq!(
            query.compare.as_ref().map(|compare| compare.offset),
            Some(CompareOffset::PreviousPeriod)
        );
    }

    #[test]
    fn every_offset_the_contract_names_parses() {
        for (spelling, offset) in [
            ("previous_period", CompareOffset::PreviousPeriod),
            ("month", CompareOffset::Month),
            ("quarter", CompareOffset::Quarter),
            ("year", CompareOffset::Year),
        ] {
            let parsed: CompareOffset =
                serde_json::from_value(serde_json::Value::String(spelling.to_owned()))
                    .unwrap_or_else(|_| panic!("should parse: {spelling}"));

            assert_eq!(parsed, offset);
        }
    }

    #[test]
    fn an_answer_states_on_the_wire_where_the_rows_behind_it_came_from() {
        let cases = [(ServedFrom::Computed, "computed")];

        for (served_from, spelling) in cases {
            let provenance = Provenance {
                executor: Executor::Semantic,
                definition_version: Some(4),
                served_from,
            };

            assert_eq!(
                serde_json::to_value(&provenance).expect("provenance serializes"),
                serde_json::json!({
                    "executor": "semantic",
                    "definition_version": 4,
                    "served_from": spelling,
                })
            );
        }
    }

    #[test]
    fn a_compared_answer_is_serialized_as_the_documented_shape() {
        let response = ValuesResponse {
            results: vec![QueryResult {
                metric: "git.commits".to_owned(),
                provenance: Provenance {
                    executor: Executor::Semantic,
                    definition_version: Some(4),
                    served_from: ServedFrom::Computed,
                },
                result: ResultBody::Values {
                    values: vec![GroupedValue {
                        subject: Some("00000000-0000-0000-0000-000000000001".to_owned()),
                        group: None,
                        value: Some(12.0),
                        compare: Some(Comparison {
                            value: Some(8.0),
                            delta: Some(4.0),
                            ratio: Some(1.5),
                        }),
                    }],
                },
            }],
        };

        assert_eq!(
            serde_json::to_value(&response).expect("the answer serializes"),
            serde_json::json!({
                "results": [{
                    "metric": "git.commits",
                    "provenance": { "served_from": "computed", "executor": "semantic", "definition_version": 4 },
                    "result": {
                        "shape": "values",
                        "values": [{
                            "subject": "00000000-0000-0000-0000-000000000001",
                            "value": 12.0,
                            "compare": { "value": 8.0, "delta": 4.0, "ratio": 1.5 },
                        }],
                    },
                }],
            })
        );
    }

    #[test]
    fn a_compared_series_carries_its_comparison_beside_the_total_and_not_the_points() {
        let response = ValuesResponse {
            results: vec![QueryResult {
                metric: "git.commits".to_owned(),
                provenance: Provenance {
                    executor: Executor::Semantic,
                    definition_version: None,
                    served_from: ServedFrom::Computed,
                },
                result: ResultBody::Series {
                    series: vec![GroupedSeries {
                        subject: Some("00000000-0000-0000-0000-000000000001".to_owned()),
                        group: None,
                        points: vec![Point {
                            date: "2026-01-05".to_owned(),
                            value: Some(9.0),
                        }],
                        total: Some(9.0),
                        compare: Some(Comparison {
                            value: None,
                            delta: None,
                            ratio: None,
                        }),
                    }],
                },
            }],
        };

        assert_eq!(
            serde_json::to_value(&response).expect("the answer serializes"),
            serde_json::json!({
                "results": [{
                    "metric": "git.commits",
                    "provenance": { "served_from": "computed", "executor": "semantic" },
                    "result": {
                        "shape": "series",
                        "series": [{
                            "subject": "00000000-0000-0000-0000-000000000001",
                            "points": [{ "date": "2026-01-05", "value": 9.0 }],
                            "total": 9.0,
                            "compare": { "value": null, "delta": null, "ratio": null },
                        }],
                    },
                }],
            })
        );
    }

    #[test]
    fn a_field_the_contract_does_not_declare_is_refused_rather_than_ignored() {
        let cases = [
            serde_json::json!({
                "metric": "git.commits",
                "subjects": { "type": "tenant" },
                "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": "total" },
                "split": { "dimensions": ["repository"], "bucket": "day" },
                "fold": "combined",
            }),
            serde_json::json!({
                "metric": "git.commits",
                "subjects": { "type": "tenant" },
                "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": "total" },
                "filters": [{ "dimension": "repository", "values": ["x"], "op": "in" }],
                "split": { "dimensions": ["repository"] },
                "fold": "combined",
            }),
            serde_json::json!({
                "metric": "git.commits",
                "subjects": { "type": "tenant" },
                "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": "total" },
                "compare": { "offset": "month", "baseline": "zero" },
                "split": { "dimensions": ["repository"] },
                "fold": "combined",
            }),
        ];

        for query in cases {
            let named = query.to_string();
            let body = serde_json::json!({ "queries": [query] });

            let parsed: Result<ValuesRequest, _> = serde_json::from_value(body);

            assert!(parsed.is_err(), "should refuse: {named}");
        }
    }

    #[test]
    fn a_capped_split_series_is_serialized_as_the_documented_shape() {
        let response = ValuesResponse {
            results: vec![QueryResult {
                metric: "git.commits".to_owned(),
                provenance: Provenance {
                    executor: Executor::Semantic,
                    definition_version: Some(1),
                    served_from: ServedFrom::Computed,
                },
                result: ResultBody::Series {
                    series: vec![
                        GroupedSeries {
                            subject: Some("00000000-0000-0000-0000-000000000001".to_owned()),
                            group: Some(Group::Dimensions {
                                dimensions: vec![GroupDimension {
                                    key: "repository".to_owned(),
                                    value: "acme/app".to_owned(),
                                    label: Some("Acme App".to_owned()),
                                }],
                            }),
                            points: vec![Point {
                                date: "2026-01-05".to_owned(),
                                value: Some(9.0),
                            }],
                            total: Some(9.0),
                            compare: None,
                        },
                        GroupedSeries {
                            subject: Some("00000000-0000-0000-0000-000000000001".to_owned()),
                            group: Some(Group::Remainder {}),
                            points: vec![Point {
                                date: "2026-01-05".to_owned(),
                                value: None,
                            }],
                            total: Some(2.0),
                            compare: None,
                        },
                    ],
                },
            }],
        };

        assert_eq!(
            serde_json::to_value(&response).expect("the answer serializes"),
            serde_json::json!({
                "results": [{
                    "metric": "git.commits",
                    "provenance": { "served_from": "computed", "executor": "semantic", "definition_version": 1 },
                    "result": {
                        "shape": "series",
                        "series": [
                            {
                                "subject": "00000000-0000-0000-0000-000000000001",
                                "group": {
                                    "type": "dimensions",
                                    "dimensions": [
                                        { "key": "repository", "value": "acme/app", "label": "Acme App" }
                                    ],
                                },
                                "points": [{ "date": "2026-01-05", "value": 9.0 }],
                                "total": 9.0,
                            },
                            {
                                "subject": "00000000-0000-0000-0000-000000000001",
                                "group": { "type": "remainder" },
                                "points": [{ "date": "2026-01-05", "value": null }],
                                "total": 2.0,
                            },
                        ],
                    },
                }],
            })
        );
    }

    #[test]
    fn a_window_total_is_serialized_as_values_and_omits_what_it_does_not_know() {
        let response = ValuesResponse {
            results: vec![QueryResult {
                metric: "git.commits".to_owned(),
                provenance: Provenance {
                    executor: Executor::Semantic,
                    definition_version: None,
                    served_from: ServedFrom::Computed,
                },
                result: ResultBody::Values {
                    values: vec![GroupedValue {
                        subject: None,
                        group: None,
                        value: None,
                        compare: None,
                    }],
                },
            }],
        };

        assert_eq!(
            serde_json::to_value(&response).expect("the answer serializes"),
            serde_json::json!({
                "results": [{
                    "metric": "git.commits",
                    "provenance": { "served_from": "computed", "executor": "semantic" },
                    "result": { "shape": "values", "values": [{ "value": null }] },
                }],
            })
        );
    }
}
