use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::super::person_names::named_persons;
use super::super::roster_names::{self, Naming, Roster};
use super::super::{AppState, forwarded_authorization, require_admin};
use super::search::PersonSearch;
use super::sort::{ActionsSort, Order, PagesSort, PeopleSort, SortKey};
use super::{
    NIL_UUID, PAGE_VIEW, SESSION_START, TABLE, VISITS, WINDOW, WindowBinds, admin_only,
    parse_range, read_error, visitors,
};
use crate::domain::date_window::Window;

const BREAKDOWN_LIMIT: u32 = 200;

fn by_page_sql(visitors: &str, order: Order<PagesSort>) -> String {
    let order = order.clause("");
    format!(
        "SELECT path, count() AS views, {visitors} AS visitors \
         FROM {TABLE} WHERE {WINDOW} AND event_name = '{PAGE_VIEW}' AND path != '' \
         GROUP BY path ORDER BY {order} LIMIT {BREAKDOWN_LIMIT}"
    )
}

#[derive(Debug)]
struct PeopleSearch<'a> {
    needle: &'a PersonSearch,
    rostered: &'a [Uuid],
}

// INVARIANT: binds follow the placeholders — the window, then a search's
// tenant, needle twice and roster hits, then the identity join's tenant.
fn people_query(
    ch: &insight_clickhouse::Client,
    binds: &WindowBinds,
    order: Order<PeopleSort>,
    search: Option<&PeopleSearch<'_>>,
) -> clickhouse::query::Query {
    let mut query = binds.query(ch, &people_sql(order, search));
    if let Some(search) = search {
        query = query
            .bind(binds.tenant.as_str())
            .bind(search.needle.as_str())
            .bind(search.needle.as_str())
            .bind(search.rostered);
    }

    query.bind(binds.tenant.as_str())
}

fn people_sql(order: Order<PeopleSort>, search: Option<&PeopleSearch<'_>>) -> String {
    let named = named_persons();
    let capped = order.clause("");
    let joined = order.clause("u.");
    let matching = search.map_or_else(String::new, |_| {
        format!(
            " AND (person_id IN (SELECT person_id FROM {named} \
             WHERE positionCaseInsensitiveUTF8(display_name, ?) > 0 \
             OR positionCaseInsensitiveUTF8(username, ?) > 0) \
             OR person_id IN ?)"
        )
    });
    format!(
        "SELECT u.person AS person_id, \
         coalesce(p.display_name, '') AS display_name, \
         coalesce(p.username, '') AS username, \
         u.visits AS visits, u.page_views AS page_views, u.last_seen AS last_seen \
         FROM (\
           SELECT person_id AS person, {VISITS} AS visits, \
           countIf(event_name = '{PAGE_VIEW}') AS page_views, \
           max(ts) AS last_ts, toString(last_ts) AS last_seen \
           FROM {TABLE} WHERE {WINDOW} AND person_id != toUUID('{NIL_UUID}'){matching} \
           GROUP BY person ORDER BY {capped} \
           LIMIT {BREAKDOWN_LIMIT}) AS u \
         LEFT JOIN {named} AS p ON p.person_id = u.person \
         ORDER BY {joined}"
    )
}

/// Excludes the two events already counted as visits.
fn actions_sql(visitors: &str, order: Order<ActionsSort>) -> String {
    let order = order.clause("");
    format!(
        "SELECT event_name, target, count() AS opens, {visitors} AS people \
         FROM {TABLE} WHERE {WINDOW} \
         AND event_name NOT IN ('{PAGE_VIEW}', '{SESSION_START}') \
         GROUP BY event_name, target ORDER BY {order} LIMIT {BREAKDOWN_LIMIT}"
    )
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct UsagePersonRow {
    #[serde(with = "clickhouse::serde::uuid")]
    person_id: Uuid,
    display_name: String,
    username: String,
    visits: u64,
    page_views: u64,
    last_seen: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UsagePerson {
    pub person_id: String,
    /// The roster's name when it lists the visitor, as People shows it; else
    /// the identity rows', empty when they hold none.
    pub display_name: String,
    /// The roster's handle when it lists the visitor; else the identity rows',
    /// empty when they hold none.
    pub username: String,
    /// The roster's address; null for a visitor the roster does not list.
    pub email: Option<String>,
    pub visits: u64,
    pub page_views: u64,
    pub last_seen: String,
}

fn named(row: UsagePersonRow, roster: &Roster) -> UsagePerson {
    let naming = Naming::of(roster.get(&row.person_id), row.display_name, row.username);

    UsagePerson {
        person_id: row.person_id.to_string(),
        display_name: naming.display_name,
        username: naming.username,
        email: naming.email,
        visits: row.visits,
        page_views: row.page_views,
        last_seen: row.last_seen,
    }
}

fn visitor_ids(rows: &[UsagePersonRow]) -> Vec<Uuid> {
    rows.iter().map(|row| row.person_id).collect()
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UsageListQuery {
    pub since: Option<String>,
    pub until: Option<String>,
    pub sort: Option<String>,
    pub direction: Option<String>,
}

impl UsageListQuery {
    fn plan<K: SortKey>(&self) -> Result<(Window, Order<K>), CanonicalError> {
        let window = parse_range(self.since.as_deref(), self.until.as_deref())?;
        let order = Order::parse(self.sort.as_deref(), self.direction.as_deref())?;

        Ok((window, order))
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UsagePeopleQuery {
    pub since: Option<String>,
    pub until: Option<String>,
    pub sort: Option<String>,
    pub direction: Option<String>,
    pub search: Option<String>,
}

impl UsagePeopleQuery {
    fn plan(&self) -> Result<(Window, Order<PeopleSort>, Option<PersonSearch>), CanonicalError> {
        let window = parse_range(self.since.as_deref(), self.until.as_deref())?;
        let order = Order::parse(self.sort.as_deref(), self.direction.as_deref())?;
        let search = PersonSearch::parse(self.search.as_deref())?;

        Ok((window, order, search))
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UsagePeopleResponse {
    pub since: String,
    pub until: String,
    pub items: Vec<UsagePerson>,
}
impl toolkit::api::api_dto::ResponseApiDto for UsagePeopleResponse {}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UsagePagesResponse {
    pub since: String,
    pub until: String,
    pub items: Vec<UsagePage>,
}
impl toolkit::api::api_dto::ResponseApiDto for UsagePagesResponse {}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UsageActionsResponse {
    pub since: String,
    pub until: String,
    pub items: Vec<UsageEvent>,
}
impl toolkit::api::api_dto::ResponseApiDto for UsageActionsResponse {}

pub async fn get_usage_people(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Query(query): Query<UsagePeopleQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let (window, order, needle) = query.plan()?;
    let binds = WindowBinds::new(&ctx, &window);
    let authorization = forwarded_authorization(&headers);

    let rostered = match &needle {
        Some(needle) => {
            roster_names::matching(&state.identity, needle.as_str(), authorization).await
        }
        None => Vec::new(),
    };
    let search = needle.as_ref().map(|needle| PeopleSearch {
        needle,
        rostered: &rostered,
    });
    let rows = people_query(&state.ch, &binds, order, search.as_ref())
        .fetch_all::<UsagePersonRow>()
        .await
        .map_err(read_error)?;

    let roster = roster_names::roster(&state.identity, &visitor_ids(&rows), authorization).await;
    let items = rows.into_iter().map(|row| named(row, &roster)).collect();

    Ok(Json(UsagePeopleResponse {
        since: binds.since,
        until: binds.until,
        items,
    }))
}

pub async fn get_usage_pages(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Query(query): Query<UsageListQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let (window, order) = query.plan::<PagesSort>()?;
    let binds = WindowBinds::new(&ctx, &window);

    let items = binds
        .query(&state.ch, &by_page_sql(&visitors(), order))
        .fetch_all::<UsagePage>()
        .await
        .map_err(read_error)?;

    Ok(Json(UsagePagesResponse {
        since: binds.since,
        until: binds.until,
        items,
    }))
}

pub async fn get_usage_actions(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Query(query): Query<UsageListQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let (window, order) = query.plan::<ActionsSort>()?;
    let binds = WindowBinds::new(&ctx, &window);

    let items = binds
        .query(&state.ch, &actions_sql(&visitors(), order))
        .fetch_all::<UsageEvent>()
        .await
        .map_err(read_error)?;

    Ok(Json(UsageActionsResponse {
        since: binds.since,
        until: binds.until,
        items,
    }))
}

#[cfg(test)]
mod tests;
