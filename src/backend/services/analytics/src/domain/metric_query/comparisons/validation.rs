//! The boundary between what a caller wrote and what a comparison reasons
//! about: parsed target ids, a metric the definitions carry, and a population
//! it can be compared within. INVARIANT: a metric declaring no cohort never
//! becomes a cohort comparison, so no read widens the population asked for.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use uuid::Uuid;

use crate::domain::compiler::request::DimensionFilter;
use crate::domain::definitions::definition::MetricDefinition;
use crate::domain::field_catalog::model::EntityType;

use super::super::catalog::MetricCatalog;
use super::super::error::QueryError;
use super::super::question::{batch_size, defined_metric, filters, person_ids, window};
use super::dto::{ComparisonQuery, ComparisonsRequest, Population};

/// The field a comparison names its people in.
pub(super) const TARGETS_FIELD: &str = "queries.targets";

/// Who a target is compared against, resolved against the metric's own
/// declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatedPopulation {
    /// Everyone sharing the metric's declared cohort with the target, under
    /// the entity type the metric declares membership by.
    DeclaredCohort {
        entity_type: EntityType,
        cohort_key: String,
    },
    /// Every person the tenant's identity mapping knows.
    Tenant,
}

#[derive(Debug, PartialEq)]
pub struct ValidatedComparison {
    pub metric_key: String,
    pub targets: Vec<Uuid>,
    pub population: ValidatedPopulation,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub filters: Vec<DimensionFilter>,
}

#[derive(Debug, PartialEq)]
pub struct ValidatedComparisons {
    pub queries: Vec<ValidatedComparison>,
}

impl ValidatedComparisons {
    /// Every person the request answers for, deduplicated across its questions.
    #[must_use]
    pub fn target_ids(&self) -> Vec<Uuid> {
        self.queries
            .iter()
            .flat_map(|query| query.targets.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

pub fn validate_request(
    catalog: &MetricCatalog,
    request: ComparisonsRequest,
) -> Result<ValidatedComparisons, QueryError> {
    batch_size(request.queries.len())?;

    let queries = request
        .queries
        .into_iter()
        .map(|query| validate_query(catalog, query))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ValidatedComparisons { queries })
}

fn validate_query(
    catalog: &MetricCatalog,
    query: ComparisonQuery,
) -> Result<ValidatedComparison, QueryError> {
    let metric_key = defined_metric(catalog, &query.metric)?;
    let targets = person_ids(TARGETS_FIELD, query.targets)?;
    let (from, to) = window(&query.time.from, &query.time.to)?;
    let filters = filters(catalog, &metric_key, query.filters)?;
    // INVARIANT: `defined_metric` refused a key the definitions do not carry.
    let Some(metric) = catalog.metric(&metric_key) else {
        return Err(QueryError::UnknownMetric { metric: metric_key });
    };
    let population = population(metric, query.population)?;

    Ok(ValidatedComparison {
        metric_key,
        targets,
        population,
        from,
        to,
        filters,
    })
}

/// INVARIANT: the cohort a comparison reads is the metric's own, never one a
/// request names, so a caller cannot ask a metric to be compared within a
/// grouping it was not defined against.
pub(in crate::domain::metric_query) fn population(
    metric: &MetricDefinition,
    population: Population,
) -> Result<ValidatedPopulation, QueryError> {
    // A comparison sets one person against the people around them. A tenant is
    // the only one of its kind in the answer, so there is no population to
    // place it in.
    if metric.entity_type == EntityType::Tenant {
        return Err(QueryError::Unanswerable {
            reason: "this metric measures the tenant, which has no peers to be compared with",
        });
    }

    match population {
        Population::Tenant {} => Ok(ValidatedPopulation::Tenant),
        Population::Cohort {} => {
            let Some(cohort_key) = metric.cohort_key.clone() else {
                return Err(QueryError::CohortUndeclared {
                    metric: metric.key.clone(),
                });
            };

            Ok(ValidatedPopulation::DeclaredCohort {
                entity_type: metric.entity_type,
                cohort_key,
            })
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::super::catalog::product_metric_catalog;
    use super::super::super::fixtures::SHIPPED_METRIC;
    use super::super::super::question::{MAX_QUERIES, MAX_SUBJECTS};
    use super::*;

    fn catalog() -> &'static MetricCatalog {
        product_metric_catalog().expect("the shipped definitions load")
    }

    fn person() -> Uuid {
        Uuid::from_u128(1)
    }

    fn query(overrides: &serde_json::Value) -> serde_json::Value {
        let mut query = serde_json::json!({
            "metric": SHIPPED_METRIC,
            "targets": [person().to_string()],
            "population": { "type": "cohort" },
            "time": { "from": "2026-01-01", "to": "2026-01-31" },
        });
        let base = query.as_object_mut().expect("an object");
        for (key, value) in overrides.as_object().expect("an object") {
            base.insert(key.clone(), value.clone());
        }
        query
    }

    fn validate(query: &serde_json::Value) -> Result<ValidatedComparisons, QueryError> {
        let request: ComparisonsRequest =
            serde_json::from_value(serde_json::json!({ "queries": [query] }))
                .expect("the wire shape parses");
        validate_request(catalog(), request)
    }

    #[test]
    fn a_cohort_comparison_reads_the_cohort_the_metric_itself_declares() {
        let validated =
            validate(&query(&serde_json::json!({}))).expect("a shipped metric compares");

        assert_eq!(
            validated.queries[0].population,
            ValidatedPopulation::DeclaredCohort {
                entity_type: EntityType::Person,
                cohort_key: "org_unit".to_owned(),
            }
        );
        assert_eq!(validated.queries[0].targets, vec![person()]);
    }

    #[test]
    fn a_tenant_comparison_names_no_cohort_at_all() {
        let validated = validate(&query(&serde_json::json!({
            "population": { "type": "tenant" },
        })))
        .expect("a tenant population needs no declaration");

        assert_eq!(validated.queries[0].population, ValidatedPopulation::Tenant);
    }

    #[test]
    fn a_metric_declaring_no_cohort_has_no_population_to_be_compared_within() {
        let undeclared = MetricDefinition {
            cohort_key: None,
            ..catalog()
                .metric(SHIPPED_METRIC)
                .expect("the metric is shipped")
                .clone()
        };

        assert!(matches!(
            population(&undeclared, Population::Cohort {})
                .expect_err("a metric declaring no cohort has no population"),
            QueryError::CohortUndeclared { .. }
        ));
        assert_eq!(
            population(&undeclared, Population::Tenant {}).ok(),
            Some(ValidatedPopulation::Tenant),
            "a tenant population needs no declaration"
        );
    }

    #[test]
    fn a_question_is_refused_when_it_names_a_metric_window_or_target_no_read_admits() {
        let cases = [
            (
                query(&serde_json::json!({ "metric": "git.not_a_shipped_metric" })),
                "a metric the definitions do not carry",
            ),
            (
                query(&serde_json::json!({ "targets": [] })),
                "a comparison for nobody",
            ),
            (
                query(&serde_json::json!({ "targets": ["nobody"] })),
                "a target that is not a person id",
            ),
            (
                query(&serde_json::json!({
                    "targets": (0..=MAX_SUBJECTS).map(|_| Uuid::new_v4().to_string()).collect::<Vec<_>>(),
                })),
                "one target past the ceiling",
            ),
            (
                query(&serde_json::json!({
                    "time": { "from": "2026-02-01", "to": "2026-01-01" },
                })),
                "a window running backwards",
            ),
            (
                query(&serde_json::json!({
                    "filters": [{ "dimension": "not_a_dimension", "values": ["x"] }],
                })),
                "a narrowing the metric cannot resolve",
            ),
        ];

        for (query, named) in cases {
            assert!(validate(&query).is_err(), "should refuse: {named}");
        }
    }

    #[test]
    fn a_request_is_refused_when_it_asks_nothing_or_asks_past_the_batch_cap() {
        let empty: ComparisonsRequest =
            serde_json::from_value(serde_json::json!({ "queries": [] })).expect("parses");
        assert!(matches!(
            validate_request(catalog(), empty).expect_err("a request asks something"),
            QueryError::NoQueries
        ));

        let queries: Vec<serde_json::Value> = (0..=MAX_QUERIES)
            .map(|_| query(&serde_json::json!({})))
            .collect();
        let overflowing: ComparisonsRequest =
            serde_json::from_value(serde_json::json!({ "queries": queries })).expect("parses");
        assert!(matches!(
            validate_request(catalog(), overflowing).expect_err("one question over the cap"),
            QueryError::TooManyQueries { .. }
        ));
    }

    #[test]
    fn the_same_target_named_twice_is_answered_for_once() {
        let validated = validate(&query(&serde_json::json!({
            "targets": [person().to_string(), person().to_string()],
        })))
        .expect("a repeated target is one target");

        assert_eq!(validated.queries[0].targets, vec![person()]);
        assert_eq!(validated.target_ids(), vec![person()]);
    }
}
