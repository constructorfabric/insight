//! What each comparison compiles to, decided before anything is read: who the
//! population is and what each of them is known by. Both are shared across the
//! questions of one request, and both arrive resolved at the compiler, which
//! looks nothing up.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::domain::compiler::request::{
    Bucket, ComparisonPopulation, ComparisonView, EntityScope, MetricQuery, ResolvedPerson,
    ViewKind,
};
use crate::domain::compiler::sql::CompiledMeasureQuery;

use super::super::catalog::MetricCatalog;
use super::super::error::QueryError;
use super::super::question::query_row_limit;
use super::pool::{cohort_pool, tenant_pool};
use super::validation::{ValidatedComparison, ValidatedComparisons, ValidatedPopulation};

// INVARIANT: a comparison reports one value per target over the whole window,
// so the peer read never folds to a bucket and this field goes unread.
const UNREAD_BUCKET: Bucket = Bucket::Day;

/// What the population pre-pass of one request has already answered. Two
/// questions over the same people commonly share a population, and its answer
/// is the same for every one of them, so it is read once and held.
#[derive(Debug, Default)]
struct Populations {
    cohorts: BTreeMap<PopulationKey, Vec<ResolvedPerson>>,
    tenant: BTreeMap<Vec<Uuid>, Vec<ResolvedPerson>>,
}

/// Everything a population read's answer depends on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PopulationKey {
    entity_type: String,
    cohort_key: String,
    targets: Vec<Uuid>,
}

/// The statements one request runs, in the order its questions were asked.
pub(super) async fn plan(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    batch: &ValidatedComparisons,
) -> Result<Vec<CompiledMeasureQuery>, QueryError> {
    let mut populations = Populations::default();
    let mut compiled = Vec::with_capacity(batch.queries.len());

    for query in &batch.queries {
        let (population, pool) = resolve(clickhouse, tenant_id, &mut populations, query).await?;
        compiled.push(compile(catalog, tenant_id, query, population, pool)?);
    }
    Ok(compiled)
}

async fn resolve(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    populations: &mut Populations,
    query: &ValidatedComparison,
) -> Result<(ComparisonPopulation, Vec<ResolvedPerson>), QueryError> {
    match &query.population {
        ValidatedPopulation::DeclaredCohort {
            entity_type,
            cohort_key,
        } => {
            let key = PopulationKey {
                entity_type: entity_type.clone(),
                cohort_key: cohort_key.clone(),
                targets: query.targets.clone(),
            };
            let pool = declared_cohort(clickhouse, tenant_id, populations, key).await?;

            Ok((
                ComparisonPopulation::DeclaredCohort {
                    cohort_key: cohort_key.clone(),
                },
                pool,
            ))
        }
        ValidatedPopulation::Tenant => {
            let pool = whole_tenant(clickhouse, tenant_id, populations, &query.targets).await?;

            Ok((ComparisonPopulation::Tenant, pool))
        }
    }
}

async fn declared_cohort(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    populations: &mut Populations,
    key: PopulationKey,
) -> Result<Vec<ResolvedPerson>, QueryError> {
    if let Some(pool) = populations.cohorts.get(&key) {
        return Ok(pool.clone());
    }

    let pool = cohort_pool(
        clickhouse,
        tenant_id,
        &key.entity_type,
        &key.cohort_key,
        &key.targets,
    )
    .await?;
    populations.cohorts.insert(key, pool.clone());
    Ok(pool)
}

async fn whole_tenant(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    populations: &mut Populations,
    targets: &[Uuid],
) -> Result<Vec<ResolvedPerson>, QueryError> {
    if let Some(pool) = populations.tenant.get(targets) {
        return Ok(pool.clone());
    }

    let pool = tenant_pool(clickhouse, tenant_id, targets).await?;
    populations.tenant.insert(targets.to_vec(), pool.clone());
    Ok(pool)
}

fn compile(
    catalog: &MetricCatalog,
    tenant_id: Uuid,
    query: &ValidatedComparison,
    population: ComparisonPopulation,
    pool: Vec<ResolvedPerson>,
) -> Result<CompiledMeasureQuery, QueryError> {
    // INVARIANT: validation refuses a metric the definitions do not carry, so
    // every planned question names one they do.
    let Some(metric) = catalog.metric(&query.metric_key) else {
        return Err(QueryError::UnknownMetric {
            metric: query.metric_key.clone(),
        });
    };

    let compiled = catalog.compile(
        metric,
        &MetricQuery {
            tenant_id: tenant_id.to_string(),
            // INVARIANT: a peer read takes its entities from its pool, so the
            // request's own scope narrows nothing here.
            entity_scope: EntityScope::Tenant,
            from: query.from,
            to: query.to,
            bucket: UNREAD_BUCKET,
            dimension_filters: query.filters.clone(),
            view: ViewKind::Comparison(ComparisonView {
                population,
                targets: query.targets.iter().map(Uuid::to_string).collect(),
                pool,
            }),
            row_limit: query_row_limit(),
        },
    )?;
    Ok(compiled)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::super::catalog::product_metric_catalog;
    use super::super::super::fixtures::{SHIPPED_METRIC, offline_clickhouse, tenant};
    use super::*;

    fn person() -> Uuid {
        Uuid::from_u128(1)
    }

    fn validated(population: ValidatedPopulation) -> ValidatedComparison {
        ValidatedComparison {
            metric_key: SHIPPED_METRIC.to_owned(),
            targets: vec![person()],
            population,
            from: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: chrono::NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
            filters: Vec::new(),
        }
    }

    fn declared() -> ValidatedPopulation {
        ValidatedPopulation::DeclaredCohort {
            entity_type: "person".to_owned(),
            cohort_key: "org_unit".to_owned(),
        }
    }

    fn pool() -> Vec<ResolvedPerson> {
        vec![ResolvedPerson {
            person_ref: person().to_string(),
            identities: vec!["dev@example.com".to_owned()],
        }]
    }

    #[test]
    fn a_cohort_comparison_reads_the_cohort_relation_and_takes_the_spread_in_one_statement() {
        let compiled = compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            &validated(declared()),
            ComparisonPopulation::DeclaredCohort {
                cohort_key: "org_unit".to_owned(),
            },
            pool(),
        )
        .expect("a shipped metric compares");

        assert!(
            compiled
                .sql
                .contains("FROM insight.metric_entity_cohorts_current")
        );
        assert!(compiled.sql.contains("    targets.entity_id AS entity_id,"));
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn a_tenant_comparison_never_reads_the_cohort_relation() {
        let compiled = compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            &validated(ValidatedPopulation::Tenant),
            ComparisonPopulation::Tenant,
            pool(),
        )
        .expect("a shipped metric compares");

        assert!(
            !compiled
                .sql
                .contains("insight.metric_entity_cohorts_current")
        );
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[tokio::test]
    async fn a_population_no_server_answers_for_refuses_the_whole_request() {
        let batch = ValidatedComparisons {
            queries: vec![validated(declared()), validated(declared())],
        };

        let outcome = plan(
            product_metric_catalog().expect("loads"),
            &offline_clickhouse(),
            tenant(),
            &batch,
        )
        .await;

        assert!(matches!(
            outcome.expect_err("a closed port cannot answer"),
            QueryError::PopulationUnresolved
        ));
    }

    /// Every shipped metric declaring a cohort answers a comparison, under
    /// either population, as one statement binding one value per placeholder.
    #[test]
    fn every_shipped_metric_that_declares_a_cohort_compiles_a_comparison() {
        let catalog = product_metric_catalog().expect("loads");
        let definitions = crate::domain::definitions::seeds::product_definitions()
            .expect("the shipped definitions are valid");
        let declaring: Vec<&str> = definitions
            .metrics
            .iter()
            .filter(|metric| metric.cohort_key.is_some())
            .map(|metric| metric.key.as_str())
            .collect();

        assert!(
            !declaring.is_empty(),
            "the shipped definitions carry a metric to compare within a cohort"
        );
        for key in declaring {
            for (population, peers) in [
                (
                    declared(),
                    ComparisonPopulation::DeclaredCohort {
                        cohort_key: "org_unit".to_owned(),
                    },
                ),
                (ValidatedPopulation::Tenant, ComparisonPopulation::Tenant),
            ] {
                let name = peers.clone();
                let query = ValidatedComparison {
                    metric_key: key.to_owned(),
                    ..validated(population)
                };

                let compiled = compile(catalog, tenant(), &query, peers, pool());

                let compiled = compiled.unwrap_or_else(|error| {
                    panic!("metric `{key}` must compare against {name:?}: {error}")
                });
                assert_eq!(
                    compiled.sql.matches('?').count(),
                    compiled.params.len(),
                    "metric `{key}` against {name:?}"
                );
            }
        }
    }
}
