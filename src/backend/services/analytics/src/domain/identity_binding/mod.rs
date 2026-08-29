//! Canonical person identifiers → the source-native identity values a dataset's
//! entity column carries. Identity policy is owned by the identity mapping
//! (`src/ingestion/dbt/macros/resolve_person_id.sql`); this module only asks it
//! questions and hands the answers on.

#![allow(dead_code)] // the semantic executor is the only caller

mod error;
mod sql;

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use uuid::Uuid;

pub use error::IdentityBindingError;

/// The source-native values standing for one person; both lists arrive sorted
/// and deduplicated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentitySet {
    pub emails: Vec<String>,
    pub account_ids: Vec<String>,
}

impl IdentitySet {
    /// Every value standing for the person, deduplicated: an identity a source
    /// recorded under both an address and an account id must not attribute its
    /// rows twice.
    #[must_use]
    pub fn values(&self) -> Vec<String> {
        let mut values: BTreeSet<&String> = self.emails.iter().collect();
        values.extend(self.account_ids.iter());
        values.into_iter().cloned().collect()
    }
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct IdentityRow {
    #[serde(with = "clickhouse::serde::uuid")]
    person_id: Uuid,
    emails: Vec<String>,
    account_ids: Vec<String>,
}

/// A person the mapping resolves nothing for is absent from the answer rather
/// than present with an empty set, which must not read as "every row".
pub async fn resolve_identities(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    person_ids: &[Uuid],
) -> Result<BTreeMap<Uuid, IdentitySet>, IdentityBindingError> {
    if person_ids.is_empty() {
        return Ok(BTreeMap::new());
    }

    let tenant = tenant_id.to_string();
    let statement = clickhouse.query(&sql::mapping_sql(sql::MappingScope::RequestedPeople));
    let email_relation = bind_tenant(statement, &tenant).bind(person_ids);
    let account_relation = bind_tenant(email_relation, &tenant).bind(person_ids);

    let rows = account_relation
        .fetch_all::<IdentityRow>()
        .await
        .map_err(|error| {
            tracing::error!(
                %error,
                %tenant_id,
                people = person_ids.len(),
                "resolving people into their source identities failed"
            );
            IdentityBindingError::MappingUnreadable
        })?;

    Ok(by_person(rows))
}

/// Every person the mapping resolves, under exactly the rules
/// [`resolve_identities`] applies to a named list. `row_limit` is the caller's
/// served ceiling plus one, so an over-large population is detected rather than
/// silently truncated.
pub async fn resolve_all_identities(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    row_limit: u64,
) -> Result<BTreeMap<Uuid, IdentitySet>, IdentityBindingError> {
    let tenant = tenant_id.to_string();
    let statement = clickhouse.query(&sql::mapping_sql(sql::MappingScope::EveryPerson));
    let email_relation = bind_tenant(statement, &tenant);
    let account_relation = bind_tenant(email_relation, &tenant);

    let rows = account_relation
        .bind(row_limit)
        .fetch_all::<IdentityRow>()
        .await
        .map_err(|error| {
            tracing::error!(
                %error,
                %tenant_id,
                "enumerating the tenant's source identities failed"
            );
            IdentityBindingError::MappingUnreadable
        })?;

    Ok(by_person(rows))
}

/// Two reads taken under one marker saw one mapping; a marker that moved says
/// the mapping may have, and nothing about which person it moved for.
pub async fn identity_epoch(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
) -> Result<u64, IdentityBindingError> {
    #[derive(Debug, Deserialize, clickhouse::Row)]
    struct EpochRow {
        epoch: u64,
    }

    clickhouse
        .query(sql::EPOCH_SQL)
        .bind(tenant_id.to_string())
        .fetch_one::<EpochRow>()
        .await
        .map(|row| row.epoch)
        .map_err(|error| {
            tracing::error!(
                %error,
                %tenant_id,
                "reading the identity mapping's epoch failed"
            );
            IdentityBindingError::EpochUnreadable
        })
}

/// INVARIANT: the mapping statement takes one tenant bind per relation only
/// while the journal tables are the relations in force; the published views
/// carry no tenant placeholder to bind.
fn bind_tenant(query: clickhouse::query::Query, tenant: &str) -> clickhouse::query::Query {
    match sql::MAPPING {
        sql::MappingRelations::InlineIdentityTables => query.bind(tenant),
        sql::MappingRelations::PublishedViews => query,
    }
}

fn by_person(rows: Vec<IdentityRow>) -> BTreeMap<Uuid, IdentitySet> {
    rows.into_iter()
        .map(|row| {
            (
                row.person_id,
                IdentitySet {
                    emails: row.emails,
                    account_ids: row.account_ids,
                },
            )
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Points at a closed port: a read that reached the network would fail.
    fn offline_client() -> insight_clickhouse::Client {
        insight_clickhouse::Client::new(insight_clickhouse::Config {
            url: "http://127.0.0.1:1".to_owned(),
            database: "identity".to_owned(),
            user: None,
            password: None,
            query_timeout: None,
            query_max_threads: None,
            query_max_memory_bytes: None,
        })
    }

    fn row(person_id: Uuid, emails: &[&str], account_ids: &[&str]) -> IdentityRow {
        IdentityRow {
            person_id,
            emails: emails.iter().map(|value| (*value).to_owned()).collect(),
            account_ids: account_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    #[tokio::test]
    async fn nobody_to_resolve_asks_the_server_nothing() {
        let resolved = resolve_identities(&offline_client(), Uuid::now_v7(), &[]).await;

        assert_eq!(resolved, Ok(BTreeMap::new()));
    }

    #[tokio::test]
    async fn an_unreadable_mapping_resolves_nobody_rather_than_everybody() {
        let resolved =
            resolve_identities(&offline_client(), Uuid::now_v7(), &[Uuid::now_v7()]).await;

        assert_eq!(resolved, Err(IdentityBindingError::MappingUnreadable));
    }

    #[tokio::test]
    async fn an_unreadable_tenant_population_resolves_nobody_rather_than_everybody() {
        let resolved = resolve_all_identities(&offline_client(), Uuid::now_v7(), 10_001).await;

        assert_eq!(resolved, Err(IdentityBindingError::MappingUnreadable));
    }

    #[tokio::test]
    async fn an_unreadable_epoch_is_its_own_failure() {
        let epoch = identity_epoch(&offline_client(), Uuid::now_v7()).await;

        assert_eq!(epoch, Err(IdentityBindingError::EpochUnreadable));
    }

    #[test]
    fn an_identity_set_is_keyed_by_the_person_it_belongs_to() {
        let alice = Uuid::from_u128(1);
        let bob = Uuid::from_u128(2);

        let resolved = by_person(vec![
            row(alice, &["alice@example.com"], &["acct-1"]),
            row(bob, &["bob@example.com", "b.park@example.com"], &[]),
        ]);

        assert_eq!(
            resolved.get(&alice),
            Some(&IdentitySet {
                emails: vec!["alice@example.com".to_owned()],
                account_ids: vec!["acct-1".to_owned()],
            })
        );
        assert_eq!(
            resolved.get(&bob).map(|set| set.emails.len()),
            Some(2),
            "both addresses a person is known by resolve to them"
        );
        assert!(
            resolved
                .get(&bob)
                .is_some_and(|set| set.account_ids.is_empty()),
            "a person no account binding names carries no account id"
        );
    }

    #[test]
    fn a_person_the_mapping_answers_nothing_for_is_absent() {
        let unresolved = Uuid::from_u128(3);

        let resolved = by_person(vec![row(Uuid::from_u128(1), &["alice@example.com"], &[])]);

        assert!(
            !resolved.contains_key(&unresolved),
            "an empty set would scope a read to every row instead of none"
        );
    }
}
