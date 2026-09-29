use serde::{Deserialize, Serialize};

use super::super::person_names::named_persons;
use super::{NIL_UUID, PAGE_VIEW, SESSION_START, TABLE, VISITS, WINDOW};

const BREAKDOWN_LIMIT: u32 = 200;

pub(super) fn by_page_sql(visitors: &str) -> String {
    format!(
        "SELECT path, count() AS views, {visitors} AS visitors \
         FROM {TABLE} WHERE {WINDOW} AND event_name = '{PAGE_VIEW}' AND path != '' \
         GROUP BY path ORDER BY views DESC LIMIT {BREAKDOWN_LIMIT}"
    )
}

/// The one read whose binds do not match the shared window: the identity join
/// scopes by tenant again, so the fourth value is bound here beside the `?`
/// that needs it rather than by the caller.
pub(super) fn people_query(
    ch: &insight_clickhouse::Client,
    tenant: &str,
    since: &str,
    until: &str,
) -> clickhouse::query::Query {
    ch.query(&people_sql())
        .bind(tenant)
        .bind(since)
        .bind(until)
        .bind(tenant)
}

/// Names come from the mirrored identity rows; a per-caller profile lookup
/// answers only for the caller's visible set, and this surface is org-wide.
pub(super) fn people_sql() -> String {
    let named = named_persons();
    format!(
        "SELECT toString(u.person) AS person_id, \
         coalesce(p.display_name, '') AS display_name, \
         coalesce(p.username, '') AS username, \
         u.visits AS visits, u.page_views AS page_views, u.last_seen AS last_seen \
         FROM (\
           SELECT person_id AS person, {VISITS} AS visits, \
           countIf(event_name = '{PAGE_VIEW}') AS page_views, \
           toString(max(ts)) AS last_seen \
           FROM {TABLE} WHERE {WINDOW} AND person_id != toUUID('{NIL_UUID}') \
           GROUP BY person ORDER BY visits DESC, page_views DESC \
           LIMIT {BREAKDOWN_LIMIT}) AS u \
         LEFT JOIN {named} AS p ON p.person_id = u.person \
         ORDER BY u.visits DESC, u.page_views DESC"
    )
}

/// Excludes the two events already counted as visits.
pub(super) fn actions_sql(visitors: &str) -> String {
    format!(
        "SELECT event_name, target, count() AS opens, {visitors} AS people \
         FROM {TABLE} WHERE {WINDOW} \
         AND event_name NOT IN ('{PAGE_VIEW}', '{SESSION_START}') \
         GROUP BY event_name, target ORDER BY opens DESC LIMIT {BREAKDOWN_LIMIT}"
    )
}

#[derive(Debug, Serialize, Deserialize, clickhouse::Row, utoipa::ToSchema)]
pub struct UsagePerson {
    pub person_id: String,
    /// Empty when the visitor has not been mirrored into the identity rows yet.
    pub display_name: String,
    /// The account handle, empty when no identity row carries one.
    pub username: String,
    pub visits: u64,
    pub page_views: u64,
    pub last_seen: String,
}

#[derive(Debug, Serialize, Deserialize, clickhouse::Row, utoipa::ToSchema)]
pub struct UsageEvent {
    pub event_name: String,
    pub target: String,
    pub opens: u64,
    pub people: u64,
}

#[derive(Debug, Serialize, Deserialize, clickhouse::Row, utoipa::ToSchema)]
pub struct UsagePage {
    pub path: String,
    pub views: u64,
    pub visitors: u64,
}

#[cfg(test)]
mod tests;
