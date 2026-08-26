//! Canonical person identifiers → the source-native identity values a dataset
//! carries.
//!
//! A request names people by person id. A dataset's entity column holds what
//! its source recorded: a lowercased email, and for a fact that knows the
//! author's account, the source's own account id. This module turns the former
//! into the latter so a read can be scoped to people, and does nothing else.
//!
//! Identity policy is not this module's. Which binding of an account is
//! current, which emails an account claims, what an email two people claim
//! resolves to, and what a binding to the excluded person means are all owned
//! by the identity mapping and stated once, in
//! `src/ingestion/dbt/macros/resolve_person_id.sql`. This module only asks that
//! mapping questions and hands the answers on; when the rules change, they
//! change there and [`sql`] follows.
//!
//! A person the mapping resolves nothing for is absent from the answer rather
//! than present with an empty set — "resolves to nobody" is the mapping's own
//! outcome for a shared address, and it must not read as "every row".

#![allow(dead_code)] // tests are this module's only callers in the crate

mod error;
mod sql;

use std::collections::BTreeMap;

use serde::Deserialize;
use uuid::Uuid;

pub use error::IdentityBindingError;

/// The source-native values that stand for one person in a dataset's entity
/// column. Both lists are deduplicated and sorted by the server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentitySet {
    pub emails: Vec<String>,
    pub account_ids: Vec<String>,
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct IdentityRow {
    #[serde(with = "clickhouse::serde::uuid")]
    person_id: Uuid,
    emails: Vec<String>,
    account_ids: Vec<String>,
}

/// The identity values each of `person_ids` is known by, in one read.
pub async fn resolve_identities(
    clickhouse: &insight_clickhouse::Client,
    tenant_id: Uuid,
    person_ids: &[Uuid],
) -> Result<BTreeMap<Uuid, IdentitySet>, IdentityBindingError> {
    if person_ids.is_empty() {
        return Ok(BTreeMap::new());
    }

    let tenant = tenant_id.to_string();
    let rows = clickhouse
        .query(&sql::mapping_sql())
        .bind(tenant.as_str())
        .bind(person_ids)
        .bind(tenant.as_str())
        .bind(person_ids)
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

/// How fresh the mapping that answered is, as a monotonic marker.
///
/// Two reads taken under one marker saw one mapping; a marker that moved says
/// the mapping may have, and says nothing about which person it moved for.
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

    /// Points at a closed port: any read that reached the network here would
    /// fail rather than answer.
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
