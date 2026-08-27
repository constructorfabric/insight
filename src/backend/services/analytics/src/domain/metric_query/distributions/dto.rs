//! The wire shape of a distribution question and its answer, asked of the
//! whole window and answered per subject.
//!
//! INVARIANT: a reading is present exactly when the question asked for it, and
//! an empty bin list or a null quantile means nothing was observed.

use serde::{Deserialize, Serialize};

use super::super::dto::{DimensionFilter, Provenance, Subjects};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DistributionsRequest {
    pub queries: Vec<DistributionQuery>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DistributionQuery {
    /// The metric key the semantic definitions carry. Only a metric whose
    /// computation is taken over its measure's own per-row values has a
    /// distribution.
    pub metric: String,
    pub subjects: Subjects,
    pub time: TimeRange,
    /// Narrows every measure the metric reads. Absent means no narrowing.
    #[serde(default)]
    pub filters: Vec<DimensionFilter>,
    /// How many bins each subject's own range is cut into. Absent means ten,
    /// unless the question asks for quantiles alone.
    #[serde(default)]
    pub bins: Option<u32>,
    /// The positions to report, each strictly between 0 and 1. Absent means
    /// none are.
    #[serde(default)]
    pub quantiles: Option<Vec<f64>>,
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
pub struct DistributionsResponse {
    /// One entry per requested query, in the order they were asked.
    pub results: Vec<DistributionResult>,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct DistributionResult {
    pub metric: String,
    pub provenance: Provenance,
    /// One entry per requested subject, in the order they were asked.
    pub subjects: Vec<SubjectDistribution>,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct SubjectDistribution {
    pub subject: String,
    /// Absent when the question asked for no histogram.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub histogram: Option<Histogram>,
    /// Absent when the question named no quantiles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantiles: Option<Vec<Quantile>>,
}

/// The subject's own range, cut into bins of equal width. INVARIANT: the last
/// bin closes on the maximum rather than opening one more.
#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct Histogram {
    /// The smallest value observed; absent when nothing was.
    pub lo: Option<f64>,
    /// The largest value observed; absent when nothing was.
    pub hi: Option<f64>,
    /// Empty when the subject was observed for no event in the window.
    pub bins: Vec<HistogramBin>,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct HistogramBin {
    pub lo: f64,
    pub hi: f64,
    pub count: u64,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct Quantile {
    /// The position asked for, strictly between 0 and 1.
    pub q: f64,
    pub value: Option<f64>,
}

impl toolkit::api::api_dto::RequestApiDto for DistributionsRequest {}
impl toolkit::api::api_dto::ResponseApiDto for DistributionsResponse {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::super::dto::Executor;
    use super::*;

    #[test]
    fn every_way_of_asking_parses_from_the_shape_it_is_documented_as() {
        let cases = [
            serde_json::json!({
                "metric": "git.pr_size",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
            }),
            serde_json::json!({
                "metric": "git.pr_size",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
                "bins": 20,
                "quantiles": [0.5, 0.9],
                "filters": [{ "dimension": "repository", "values": ["acme/app"] }],
            }),
            serde_json::json!({
                "metric": "git.pr_size",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
                "quantiles": [0.5],
            }),
        ];

        for query in cases {
            let named = query.to_string();
            let body = serde_json::json!({ "queries": [query] });

            let parsed: Result<DistributionsRequest, _> = serde_json::from_value(body);

            assert!(parsed.is_ok(), "should parse: {named}");
        }
    }

    #[test]
    fn a_field_the_contract_does_not_declare_is_refused_rather_than_ignored() {
        let cases = [
            serde_json::json!({
                "metric": "git.pr_size",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": "total" },
            }),
            serde_json::json!({
                "metric": "git.pr_size",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
                "fold": "per_subject",
            }),
            serde_json::json!({
                "metric": "git.pr_size",
                "subjects": { "type": "persons", "ids": ["00000000-0000-0000-0000-000000000001"] },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
                "percentiles": [0.5],
            }),
        ];

        for query in cases {
            let named = query.to_string();
            let body = serde_json::json!({ "queries": [query] });

            let parsed: Result<DistributionsRequest, _> = serde_json::from_value(body);

            assert!(parsed.is_err(), "should refuse: {named}");
        }
    }

    #[test]
    fn an_answer_carrying_both_readings_is_serialized_as_the_documented_shape() {
        let response = DistributionsResponse {
            results: vec![DistributionResult {
                metric: "git.pr_size".to_owned(),
                provenance: Provenance {
                    executor: Executor::Semantic,
                    definition_version: Some(2),
                },
                subjects: vec![SubjectDistribution {
                    subject: "00000000-0000-0000-0000-000000000001".to_owned(),
                    histogram: Some(Histogram {
                        lo: Some(0.0),
                        hi: Some(10.0),
                        bins: vec![
                            HistogramBin {
                                lo: 0.0,
                                hi: 5.0,
                                count: 3,
                            },
                            HistogramBin {
                                lo: 5.0,
                                hi: 10.0,
                                count: 1,
                            },
                        ],
                    }),
                    quantiles: Some(vec![Quantile {
                        q: 0.5,
                        value: Some(4.0),
                    }]),
                }],
            }],
        };

        assert_eq!(
            serde_json::to_value(&response).expect("the answer serializes"),
            serde_json::json!({
                "results": [{
                    "metric": "git.pr_size",
                    "provenance": { "executor": "semantic", "definition_version": 2 },
                    "subjects": [{
                        "subject": "00000000-0000-0000-0000-000000000001",
                        "histogram": {
                            "lo": 0.0,
                            "hi": 10.0,
                            "bins": [
                                { "lo": 0.0, "hi": 5.0, "count": 3 },
                                { "lo": 5.0, "hi": 10.0, "count": 1 },
                            ],
                        },
                        "quantiles": [{ "q": 0.5, "value": 4.0 }],
                    }],
                }],
            })
        );
    }

    #[test]
    fn a_reading_the_question_did_not_ask_for_is_absent_and_an_unobserved_one_is_empty() {
        let response = DistributionsResponse {
            results: vec![DistributionResult {
                metric: "git.pr_size".to_owned(),
                provenance: Provenance {
                    executor: Executor::Semantic,
                    definition_version: None,
                },
                subjects: vec![SubjectDistribution {
                    subject: "00000000-0000-0000-0000-000000000001".to_owned(),
                    histogram: Some(Histogram {
                        lo: None,
                        hi: None,
                        bins: Vec::new(),
                    }),
                    quantiles: None,
                }],
            }],
        };

        assert_eq!(
            serde_json::to_value(&response).expect("the answer serializes"),
            serde_json::json!({
                "results": [{
                    "metric": "git.pr_size",
                    "provenance": { "executor": "semantic" },
                    "subjects": [{
                        "subject": "00000000-0000-0000-0000-000000000001",
                        "histogram": { "lo": null, "hi": null, "bins": [] },
                    }],
                }],
            })
        );
    }
}
