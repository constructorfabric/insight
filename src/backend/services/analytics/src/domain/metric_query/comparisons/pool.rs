//! The people a comparison evaluates, as the (person, identity) pairs the
//! compiler takes: a cohort names members by person reference while a dataset
//! keys rows by source identity. INVARIANT: a population overflowing its
//! ceiling is refused, because part of a population is not the population.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use uuid::Uuid;

use crate::domain::compiler::comparison::compile_cohort_members_query;
use crate::domain::compiler::request::{CohortMembersQuery, ResolvedPerson};
use crate::domain::identity_binding::{IdentitySet, resolve_all_identities, resolve_identities};

use super::super::error::QueryError;
use super::super::execute::fetch;
use super::super::question::{query_row_limit, row_limit};
use super::validation::TARGETS_FIELD;

/// One member of the population, as the cohort relation names them.
#[derive(Debug, Deserialize)]
struct CohortMemberRow {
    entity_id: String,
}

/// Everyone who shares a declared cohort with one of the targets, with the
/// identities each of them is known by.
pub(super) async fn cohort_pool(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    entity_type: &str,
    cohort_key: &str,
    targets: &[Uuid],
) -> Result<Vec<ResolvedPerson>, QueryError> {
    if targets.is_empty() {
        return Err(QueryError::NoSubjects {
            field: TARGETS_FIELD,
        });
    }

    let query = CohortMembersQuery {
        tenant_id: tenant_id.to_string(),
        entity_type: entity_type.to_owned(),
        cohort_key: cohort_key.to_owned(),
        targets: targets.iter().map(Uuid::to_string).collect(),
        row_limit: query_row_limit(),
    };
    let compiled = compile_cohort_members_query(&query)?;

    let rows = fetch::<CohortMemberRow>(clickhouse, &compiled, "metric-comparisons:population")
        .await
        .map_err(|error| {
            tracing::warn!(%error, "reading a comparison's cohort membership failed");
            QueryError::PopulationUnresolved
        })?;
    if rows.len() > row_limit() {
        return Err(QueryError::PopulationTooLarge { limit: row_limit() });
    }

    let members = population(rows.into_iter().map(|row| row.entity_id), targets);
    let identities = resolve_identities(clickhouse, tenant_id, &person_ids(&members))
        .await
        .map_err(|error| {
            tracing::warn!(%error, "resolving a comparison's population failed");
            QueryError::PopulationUnresolved
        })?;
    Ok(pool(members, &identities))
}

/// Every person the identity mapping knows in this tenant, with the identities
/// each of them is known by.
pub(super) async fn tenant_pool(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    targets: &[Uuid],
) -> Result<Vec<ResolvedPerson>, QueryError> {
    if targets.is_empty() {
        return Err(QueryError::NoSubjects {
            field: TARGETS_FIELD,
        });
    }

    let identities = resolve_all_identities(clickhouse, tenant_id, query_row_limit())
        .await
        .map_err(|error| {
            tracing::warn!(%error, "enumerating a comparison's population failed");
            QueryError::PopulationUnresolved
        })?;
    if identities.len() > row_limit() {
        return Err(QueryError::PopulationTooLarge { limit: row_limit() });
    }

    let members = population(identities.keys().map(Uuid::to_string), targets);
    Ok(pool(members, &identities))
}

/// The population a comparison evaluates, keyed by the person reference the
/// statement joins on, with every target admitted to it.
// INVARIANT: a reference is carried as the population spelled it, because the
// read joins the cohort relation's own `entity_id`; the parsed id is only what
// the identity mapping is asked about.
fn population(
    members: impl Iterator<Item = String>,
    targets: &[Uuid],
) -> BTreeMap<String, Option<Uuid>> {
    let mut population: BTreeMap<String, Option<Uuid>> = members
        .map(|person_ref| {
            let person_id = Uuid::parse_str(&person_ref).ok();
            (person_ref, person_id)
        })
        .collect();
    for target in targets {
        population
            .entry(target.to_string())
            .or_insert(Some(*target));
    }
    population
}

fn person_ids(population: &BTreeMap<String, Option<Uuid>>) -> Vec<Uuid> {
    population
        .values()
        .filter_map(|person_id| *person_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The population with each member's identities attached. INVARIANT: a member
/// the mapping answers nothing for keeps their place with no identity, joins
/// no row, and so is left out of the population size.
fn pool(
    population: BTreeMap<String, Option<Uuid>>,
    identities: &BTreeMap<Uuid, IdentitySet>,
) -> Vec<ResolvedPerson> {
    population
        .into_iter()
        .map(|(person_ref, person_id)| ResolvedPerson {
            identities: person_id
                .and_then(|person_id| identities.get(&person_id))
                .map(IdentitySet::values)
                .unwrap_or_default(),
            person_ref,
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::super::fixtures::{offline_clickhouse, tenant};
    use super::*;

    fn identity_set(emails: &[&str], account_ids: &[&str]) -> IdentitySet {
        IdentitySet {
            emails: emails.iter().map(|value| (*value).to_owned()).collect(),
            account_ids: account_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    fn built(
        members: &[&str],
        targets: &[Uuid],
        known: &[(Uuid, IdentitySet)],
    ) -> Vec<ResolvedPerson> {
        let population = population(members.iter().map(|member| (*member).to_owned()), targets);
        let identities: BTreeMap<Uuid, IdentitySet> = known.iter().cloned().collect();
        pool(population, &identities)
    }

    #[test]
    fn every_member_enters_the_pool_with_the_identities_they_are_known_by() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);

        let pool = built(
            &[first.to_string().as_str(), second.to_string().as_str()],
            &[first],
            &[
                (first, identity_set(&["one@example.com"], &["acct-1"])),
                (second, identity_set(&["two@example.com"], &[])),
            ],
        );

        assert_eq!(
            pool,
            vec![
                ResolvedPerson {
                    person_ref: first.to_string(),
                    identities: vec!["acct-1".to_owned(), "one@example.com".to_owned()],
                },
                ResolvedPerson {
                    person_ref: second.to_string(),
                    identities: vec!["two@example.com".to_owned()],
                },
            ]
        );
    }

    #[test]
    fn a_target_the_mapping_answers_nothing_for_is_pooled_with_no_identity_at_all() {
        let unresolved = Uuid::from_u128(9);

        let pool = built(&[], &[unresolved], &[]);

        assert_eq!(
            pool,
            vec![ResolvedPerson {
                person_ref: unresolved.to_string(),
                identities: Vec::new(),
            }],
            "a target absent from the mapping joins no row and is left out of the spread"
        );
    }

    #[test]
    fn a_target_the_population_already_names_is_pooled_once() {
        let target = Uuid::from_u128(1);

        let pool = built(
            &[target.to_string().as_str()],
            &[target],
            &[(target, identity_set(&["dev@example.com"], &[]))],
        );

        assert_eq!(pool.len(), 1);
        assert_eq!(pool[0].identities, vec!["dev@example.com".to_owned()]);
    }

    /// The reference a declared-cohort read joins on is the cohort relation's
    /// own `entity_id`; a member it spells differently from a canonical UUID
    /// keeps that spelling, and is asked about under the id it parses to.
    #[test]
    fn a_member_reference_is_carried_as_the_population_spelled_it() {
        let member = Uuid::from_u128(1);
        let spelled = member.to_string().to_uppercase();

        let population = population([spelled.clone()].into_iter(), &[]);

        assert_eq!(person_ids(&population), vec![member]);
        assert_eq!(
            pool(
                population,
                &BTreeMap::from([(member, identity_set(&["dev@example.com"], &[]))])
            ),
            vec![ResolvedPerson {
                person_ref: spelled,
                identities: vec!["dev@example.com".to_owned()],
            }]
        );
    }

    #[test]
    fn a_member_reference_no_person_id_parses_from_is_asked_about_for_nobody() {
        let population = population(["not-a-person".to_owned()].into_iter(), &[]);

        assert!(person_ids(&population).is_empty());
        assert_eq!(
            pool(population, &BTreeMap::new()),
            vec![ResolvedPerson {
                person_ref: "not-a-person".to_owned(),
                identities: Vec::new(),
            }]
        );
    }

    #[tokio::test]
    async fn a_comparison_with_no_target_asks_the_server_nothing() {
        for outcome in [
            cohort_pool(&offline_clickhouse(), tenant(), "person", "org_unit", &[]).await,
            tenant_pool(&offline_clickhouse(), tenant(), &[]).await,
        ] {
            assert!(matches!(
                outcome.expect_err("no target names no population"),
                QueryError::NoSubjects { .. }
            ));
        }
    }

    #[tokio::test]
    async fn a_population_no_server_answers_for_resolves_to_no_pool() {
        let target = Uuid::from_u128(1);

        for outcome in [
            cohort_pool(
                &offline_clickhouse(),
                tenant(),
                "person",
                "org_unit",
                &[target],
            )
            .await,
            tenant_pool(&offline_clickhouse(), tenant(), &[target]).await,
        ] {
            assert!(matches!(
                outcome.expect_err("a closed port cannot answer"),
                QueryError::PopulationUnresolved
            ));
        }
    }
}
