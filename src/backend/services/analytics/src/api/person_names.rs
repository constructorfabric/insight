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
    if ids.is_empty() {
        return HashMap::new();
    }

    match ch
        .query(&aggregate(" AND person_id IN ?"))
        .bind(tenant.to_string())
        .bind(ids)
        .fetch_all::<PersonName>()
        .await
    {
        Ok(found) => found
            .into_iter()
            .map(|name| (name.person_id, name))
            .collect(),
        Err(error) => {
            tracing::warn!(error = %error, "naming people from the identity rows failed");
            HashMap::new()
        }
    }
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
