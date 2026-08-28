//! The wire shape of a comparison question and its answer. A comparison is
//! asked of the whole window, so it names no grain, no split and no fold.
//!
//! INVARIANT: the answer is aggregates only — nothing in it names a member of
//! the population a target was compared against.

use serde::{Deserialize, Serialize};

use super::super::dto::{DimensionFilter, Provenance};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ComparisonsRequest {
    pub queries: Vec<ComparisonQuery>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ComparisonQuery {
    /// The metric key the semantic definitions carry, such as `git.commits`.
    pub metric: String,
    /// The people the answer reports a value for.
    pub targets: Vec<String>,
    pub population: Population,
    pub time: TimeRange,
    /// Narrows every measure the metric reads, for the targets and the
    /// population alike. Absent means no narrowing.
    #[serde(default)]
    pub filters: Vec<DimensionFilter>,
}

/// Who a target is compared against, internally tagged on `type`. `cohort`
/// takes the metric's own declared cohort; a metric that declares none has no
/// cohort to compare within.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Population {
    Cohort {},
    Tenant {},
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
pub struct ComparisonsResponse {
    /// One entry per requested query, in the order they were asked.
    pub results: Vec<ComparisonResult>,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ComparisonResult {
    pub metric: String,
    pub provenance: Provenance,
    /// One entry per requested target, in the order they were asked.
    pub targets: Vec<TargetComparison>,
}

/// INVARIANT: the population is the one this target sits in, which under a
/// declared cohort is that target's own and need not be another target's.
#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct TargetComparison {
    pub subject: String,
    pub value: Option<f64>,
    pub population: PopulationSpread,
}

/// The spread of the population, withheld below the disclosure floor. The
/// size is reported whatever it is, so a consumer can say why the rest is
/// absent.
#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct PopulationSpread {
    /// How many of the population were observed at all.
    pub n: u64,
    pub p25: Option<f64>,
    pub median: Option<f64>,
    pub p75: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl toolkit::api::api_dto::RequestApiDto for ComparisonsRequest {}
impl toolkit::api::api_dto::ResponseApiDto for ComparisonsResponse {}

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
                "targets": ["00000000-0000-0000-0000-000000000001"],
                "population": { "type": "cohort" },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
            }),
            serde_json::json!({
                "metric": "git.commits",
                "targets": [
                    "00000000-0000-0000-0000-000000000001",
                    "00000000-0000-0000-0000-000000000002",
                ],
                "population": { "type": "tenant" },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
                "filters": [{ "dimension": "repository", "values": ["acme/app"] }],
            }),
        ];

        for query in cases {
            let named = query.to_string();
            let body = serde_json::json!({ "queries": [query] });

            let parsed: Result<ComparisonsRequest, _> = serde_json::from_value(body);

            assert!(parsed.is_ok(), "should parse: {named}");
        }
    }

    #[test]
    fn a_field_the_contract_does_not_declare_is_refused_rather_than_ignored() {
        let cases = [
            serde_json::json!({
                "metric": "git.commits",
                "targets": ["00000000-0000-0000-0000-000000000001"],
                "population": { "type": "cohort" },
                "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": "total" },
            }),
            serde_json::json!({
                "metric": "git.commits",
                "targets": ["00000000-0000-0000-0000-000000000001"],
                "population": { "type": "cohort", "cohort_key": "org_unit" },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
            }),
            serde_json::json!({
                "metric": "git.commits",
                "targets": ["00000000-0000-0000-0000-000000000001"],
                "population": { "type": "cohort" },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
                "fold": "per_subject",
            }),
            serde_json::json!({
                "metric": "git.commits",
                "targets": ["00000000-0000-0000-0000-000000000001"],
                "population": { "type": "cohort" },
                "time": { "from": "2026-01-01", "to": "2026-01-31" },
                "split": { "dimensions": ["repository"] },
            }),
        ];

        for query in cases {
            let named = query.to_string();
            let body = serde_json::json!({ "queries": [query] });

            let parsed: Result<ComparisonsRequest, _> = serde_json::from_value(body);

            assert!(parsed.is_err(), "should refuse: {named}");
        }
    }

    #[test]
    fn an_answer_is_serialized_as_the_documented_shape() {
        let response = ComparisonsResponse {
            results: vec![ComparisonResult {
                metric: "git.commits".to_owned(),
                provenance: Provenance {
                    executor: Executor::Semantic,
                    definition_version: Some(4),
                    served_from: ServedFrom::Computed,
                },
                targets: vec![TargetComparison {
                    subject: "00000000-0000-0000-0000-000000000001".to_owned(),
                    value: Some(12.0),
                    population: PopulationSpread {
                        n: 9,
                        p25: Some(4.0),
                        median: Some(7.0),
                        p75: Some(11.0),
                        min: Some(1.0),
                        max: Some(20.0),
                    },
                }],
            }],
        };

        assert_eq!(
            serde_json::to_value(&response).expect("the answer serializes"),
            serde_json::json!({
                "results": [{
                    "metric": "git.commits",
                    "provenance": { "served_from": "computed", "executor": "semantic", "definition_version": 4 },
                    "targets": [{
                        "subject": "00000000-0000-0000-0000-000000000001",
                        "value": 12.0,
                        "population": {
                            "n": 9,
                            "p25": 4.0,
                            "median": 7.0,
                            "p75": 11.0,
                            "min": 1.0,
                            "max": 20.0,
                        },
                    }],
                }],
            })
        );
    }

    #[test]
    fn a_population_under_the_floor_reports_its_size_and_no_statistic() {
        let response = ComparisonsResponse {
            results: vec![ComparisonResult {
                metric: "git.commits".to_owned(),
                provenance: Provenance {
                    executor: Executor::Semantic,
                    definition_version: None,
                    served_from: ServedFrom::Computed,
                },
                targets: vec![TargetComparison {
                    subject: "00000000-0000-0000-0000-000000000001".to_owned(),
                    value: None,
                    population: PopulationSpread {
                        n: 2,
                        p25: None,
                        median: None,
                        p75: None,
                        min: None,
                        max: None,
                    },
                }],
            }],
        };

        assert_eq!(
            serde_json::to_value(&response).expect("the answer serializes"),
            serde_json::json!({
                "results": [{
                    "metric": "git.commits",
                    "provenance": { "served_from": "computed", "executor": "semantic" },
                    "targets": [{
                        "subject": "00000000-0000-0000-0000-000000000001",
                        "value": null,
                        "population": {
                            "n": 2,
                            "p25": null,
                            "median": null,
                            "p75": null,
                            "min": null,
                            "max": null,
                        },
                    }],
                }],
            })
        );
    }
}
