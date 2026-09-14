//! Naming a person from the mirrored identity rows.
//!
//! Two shapes over one aggregate: [`named_persons`] for a read model that joins
//! names onto rows it already reads out of ClickHouse, and [`lookup`] for one
//! that holds its rows elsewhere and needs names for a page of ids.

use std::collections::HashMap;

use serde::Deserialize;
use uuid::Uuid;

/// A `person_id → display_name / username` relation for one tenant, taking one
/// bound tenant parameter. Names come from the mirrored rows: a per-caller
/// profile lookup answers only for the caller's visible set, and these surfaces
/// are org-wide.
pub(crate) fn named_persons() -> String {
    format!("({})", aggregate(""))
}

#[derive(Debug, Deserialize, clickhouse::Row)]
pub(crate) struct PersonName {
    #[serde(with = "clickhouse::serde::uuid")]
    person_id: Uuid,
    pub(crate) display_name: String,
    pub(crate) username: String,
}

/// Names for exactly these people.
///
/// A failed lookup names nobody rather than failing the caller's read: the rows
/// being named are the thing on screen.
pub(crate) async fn lookup(
    ch: &insight_clickhouse::Client,
    tenant: Uuid,
    ids: &[Uuid],
) -> HashMap<Uuid, PersonName> {
    lookup_bounded(ch, tenant, ids, |query| query).await
}

/// The same lookup under the caller's own resource limits.
///
/// A read that names rows on a bounded surface has to be bounded too: the
/// aggregate scans the identity journal, which carries no index on the tenant.
pub(crate) async fn lookup_bounded(
    ch: &insight_clickhouse::Client,
    tenant: Uuid,
    ids: &[Uuid],
    bound: impl FnOnce(clickhouse::query::Query) -> clickhouse::query::Query,
) -> HashMap<Uuid, PersonName> {
    if ids.is_empty() {
        return HashMap::new();
    }

    match bound(ch.query(&aggregate(" AND person_id IN ?")))
        .bind(tenant.to_string())
        .bind(ids)
        .fetch_all::<PersonName>()
        .await
    {
        Ok(found) => by_person(found),
        Err(error) => {
            tracing::warn!(error = %error, "naming people from the identity rows failed");
            HashMap::new()
        }
    }
}

#[cfg(test)]
impl PersonName {
    /// The id is the map key, so a name a caller stands up carries none.
    pub(crate) fn named(display_name: &str, username: &str) -> Self {
        Self {
            person_id: Uuid::nil(),
            display_name: display_name.to_owned(),
            username: username.to_owned(),
        }
    }
}

fn by_person(found: Vec<PersonName>) -> HashMap<Uuid, PersonName> {
    found
        .into_iter()
        .map(|name| (name.person_id, name))
        .collect()
}

/// INVARIANT: every selected name coalesces to `''`. A `nullIf` left uncovered
/// makes the column `Nullable(String)`, which the row type refuses to decode —
/// and the refusal surfaces only against a real server.
///
/// INVARIANT: `person` narrows the rows the aggregate runs over, so it belongs
/// before the `GROUP BY` — after it, the argMax state is built for the whole
/// tenant and then discarded. The identity log carries no index on the tenant,
/// which makes that the difference between a scan and a seek.
fn aggregate(person: &str) -> String {
    format!(
        "SELECT person_id, \
         coalesce(\
           nullIf(argMaxIf(value_effective, (created_at, id), value_type = 'display_name'), ''), \
           nullIf(trimBoth(concat(\
             coalesce(argMaxIf(value_effective, (created_at, id), value_type = 'first_name'), ''), \
             ' ', \
             coalesce(argMaxIf(value_effective, (created_at, id), value_type = 'last_name'), '') \
           )), ''), \
           '' \
         ) AS display_name, \
         coalesce(nullIf(argMaxIf(value_effective, (created_at, id), value_type = 'username'), ''), '') AS username \
         FROM identity.identity_persons \
         WHERE value_type IN ('display_name', 'first_name', 'last_name', 'username') \
         AND insight_tenant_id = toUUID(?){person} \
         GROUP BY person_id"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn nobody_to_name_asks_the_server_nothing() {
        // The client points at a closed port: reaching the network here would
        // fail rather than return empty.
        let names = lookup(&offline_client(), Uuid::now_v7(), &[]).await;

        assert!(names.is_empty());
    }

    #[test]
    fn a_name_is_keyed_by_the_person_it_belongs_to() {
        let alice = Uuid::now_v7();
        let bob = Uuid::now_v7();
        let mut found = vec![
            PersonName::named("Alice Example", "alice"),
            PersonName::named("Bob Park", "bob"),
        ];
        found[0].person_id = alice;
        found[1].person_id = bob;

        let names = by_person(found);

        assert_eq!(
            names.get(&alice).map(|n| n.username.as_str()),
            Some("alice")
        );
        assert_eq!(
            names.get(&bob).map(|n| n.display_name.as_str()),
            Some("Bob Park")
        );
    }

    #[test]
    fn the_relation_binds_the_tenant_and_nothing_else() {
        assert_eq!(named_persons().matches('?').count(), 1);
    }

    #[test]
    fn the_lookup_binds_the_tenant_and_the_person_set() {
        assert_eq!(aggregate(" AND person_id IN ?").matches('?').count(), 2);
    }

    /// A `nullIf` the outer `coalesce` does not cover leaves the column
    /// `Nullable(String)`, and the row type decodes it as `String` — a mismatch
    /// no offline test sees, because it is raised by the server.
    #[test]
    fn every_name_the_aggregate_selects_is_never_null() {
        let sql: String = named_persons()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();

        assert!(sql.contains(",'')ASdisplay_name"), "{sql}");
        assert!(sql.contains(",'')ASusername"), "{sql}");
    }

    #[test]
    fn the_person_filter_sits_before_the_grouping() {
        let sql = aggregate(" AND person_id IN ?");

        assert!(
            sql.find("person_id IN ?") < sql.find("GROUP BY"),
            "filtering after the GROUP BY aggregates the whole tenant: {sql}"
        );
    }
}
