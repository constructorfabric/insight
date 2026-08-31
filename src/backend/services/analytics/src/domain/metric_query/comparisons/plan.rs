//! What each comparison compiles to, decided before anything is read: who the
//! population is and what each of them is known by. Both are shared across the
//! questions of one request, and both arrive resolved at the compiler, which
//! looks nothing up.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::domain::compiler::request::{
    Bucket, ComparisonPopulation, ComparisonView, EntityScope, MetricQuery, ResolvedPerson,
    ViewKind, any_identity_resolved,
};
use crate::domain::compiler::sql::CompiledMeasureQuery;
use crate::domain::field_catalog::model::EntityType;

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
    entity_type: EntityType,
    cohort_key: String,
    targets: Vec<Uuid>,
}

#[derive(Debug, PartialEq)]
pub(super) enum PlannedComparison {
    /// The one statement a comparison runs.
    Read(CompiledMeasureQuery),
    /// The mapping knows no identity for anyone in the population, so no row
    /// can carry a value for any of them: every target is unobserved and the
    /// spread is taken over nobody.
    NoIdentities,
}

/// The statements one request runs, in the order its questions were asked.
pub(super) async fn plan(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    batch: &ValidatedComparisons,
) -> Result<Vec<PlannedComparison>, QueryError> {
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
                entity_type: *entity_type,
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
        key.entity_type,
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
) -> Result<PlannedComparison, QueryError> {
    // INVARIANT: validation refuses a metric the definitions do not carry, so
    // every planned question names one they do.
    let Some(metric) = catalog.metric(&query.metric_key) else {
        return Err(QueryError::UnknownMetric {
            metric: query.metric_key.clone(),
        });
    };
    if !any_identity_resolved(&pool) {
        return Ok(PlannedComparison::NoIdentities);
    }

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
    Ok(PlannedComparison::Read(compiled))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::domain::compiler::sql::{CompiledMeasureQuery, QueryParam};

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
            entity_type: EntityType::Person,
            cohort_key: "org_unit".to_owned(),
        }
    }

    fn pool() -> Vec<ResolvedPerson> {
        vec![ResolvedPerson {
            person_ref: person().to_string(),
            identities: vec!["dev@example.com".to_owned()],
        }]
    }

    /// The one member of a pool the mapping answered nothing for.
    fn unmapped(person_id: Uuid) -> ResolvedPerson {
        ResolvedPerson {
            person_ref: person_id.to_string(),
            identities: Vec::new(),
        }
    }

    fn read(planned: PlannedComparison, named: &str) -> CompiledMeasureQuery {
        match planned {
            PlannedComparison::Read(compiled) => compiled,
            PlannedComparison::NoIdentities => panic!("{named} has somebody to read"),
        }
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
        .map(|planned| read(planned, "a cohort comparison"))
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
        .map(|planned| read(planned, "a tenant comparison"))
        .expect("a shipped metric compares");

        assert!(
            !compiled
                .sql
                .contains("insight.metric_entity_cohorts_current")
        );
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    /// A target the mapping knows nothing about joins no row, exactly as the
    /// pool builder's own rule says: it is left out of the spread, and the
    /// peers around it are still read.
    #[test]
    fn a_target_the_mapping_answers_nothing_for_is_compared_against_the_peers_that_resolved() {
        let silent = Uuid::from_u128(9);
        let query = ValidatedComparison {
            targets: vec![silent],
            ..validated(ValidatedPopulation::Tenant)
        };

        let compiled = compile(
            product_metric_catalog().expect("loads"),
            tenant(),
            &query,
            ComparisonPopulation::Tenant,
            [vec![unmapped(silent)], pool()].concat(),
        )
        .map(|planned| read(planned, "a comparison with a mapped peer"))
        .expect("a population one of whose members resolved is read");

        assert!(
            compiled
                .params
                .contains(&QueryParam::Text("dev@example.com".to_owned())),
            "the peer that resolved is pooled, so a spread is still taken"
        );
        assert_eq!(
            compiled
                .params
                .iter()
                .filter(|param| **param == QueryParam::Text(silent.to_string()))
                .count(),
            1,
            "the unmapped target is named once, as a target, and pools no pair"
        );
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    /// A comparison the mapping answers nothing for anyone in reads as no data,
    /// not as a bad request.
    #[test]
    fn a_population_no_identity_resolves_for_is_answered_without_a_read() {
        for population in [
            (
                declared(),
                ComparisonPopulation::DeclaredCohort {
                    cohort_key: "org_unit".to_owned(),
                },
            ),
            (ValidatedPopulation::Tenant, ComparisonPopulation::Tenant),
        ] {
            let (validated_population, peers) = population;
            let named = format!("{peers:?}");

            let planned = compile(
                product_metric_catalog().expect("loads"),
                tenant(),
                &validated(validated_population),
                peers,
                vec![unmapped(person())],
            )
            .unwrap_or_else(|error| panic!("should be answerable: {named} — {error}"));

            assert_eq!(planned, PlannedComparison::NoIdentities, "{named}");
        }
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

                let planned = compiled.unwrap_or_else(|error| {
                    panic!("metric `{key}` must compare against {name:?}: {error}")
                });
                let compiled = read(planned, key);
                assert_eq!(
                    compiled.sql.matches('?').count(),
                    compiled.params.len(),
                    "metric `{key}` against {name:?}"
                );
            }
        }
    }
}
