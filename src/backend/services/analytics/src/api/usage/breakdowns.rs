use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::super::person_names::named_persons;
use super::super::{AppState, require_admin};
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

/// The one read whose binds do not match the shared window: the identity join
/// scopes by tenant again, so the fourth value is bound here beside the `?`
/// that needs it rather than by the caller.
fn people_query(
    ch: &insight_clickhouse::Client,
    binds: &WindowBinds,
    order: Order<PeopleSort>,
) -> clickhouse::query::Query {
    binds
        .query(ch, &people_sql(order))
        .bind(binds.tenant.as_str())
}

/// Names come from the mirrored identity rows; a per-caller profile lookup
/// answers only for the caller's visible set, and this surface is org-wide.
fn people_sql(order: Order<PeopleSort>) -> String {
    let named = named_persons();
    let capped = order.clause("");
    let joined = order.clause("u.");
    format!(
        "SELECT toString(u.person) AS person_id, \
         coalesce(p.display_name, '') AS display_name, \
         coalesce(p.username, '') AS username, \
         u.visits AS visits, u.page_views AS page_views, u.last_seen AS last_seen \
         FROM (\
           SELECT person_id AS person, {VISITS} AS visits, \
           countIf(event_name = '{PAGE_VIEW}') AS page_views, \
           max(ts) AS last_ts, toString(last_ts) AS last_seen \
           FROM {TABLE} WHERE {WINDOW} AND person_id != toUUID('{NIL_UUID}') \
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
    Query(query): Query<UsageListQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let (window, order) = query.plan::<PeopleSort>()?;
    let binds = WindowBinds::new(&ctx, &window);

    let items = people_query(&state.ch, &binds, order)
        .fetch_all::<UsagePerson>()
        .await
        .map_err(read_error)?;

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
