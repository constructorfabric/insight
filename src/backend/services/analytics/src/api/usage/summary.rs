use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::super::{AppState, require_admin};
use super::breakdowns::{
    UsageEvent, UsagePage, UsagePerson, actions_sql, by_page_sql, people_query,
};
use super::{NIL_UUID, PAGE_VIEW, TABLE, UsageRangeQuery, VISITS, WINDOW, admin_only, read_error};

fn totals_sql(visitors: &str) -> String {
    format!(
        "SELECT {VISITS} AS visits, {visitors} AS visitors, \
         countIf(event_name = '{PAGE_VIEW}') AS page_views \
         FROM {TABLE} WHERE {WINDOW}"
    )
}

fn by_day_sql(visitors: &str) -> String {
    format!(
        "SELECT toString(toDate(ts)) AS day, {VISITS} AS visits, \
         {visitors} AS visitors \
         FROM {TABLE} WHERE {WINDOW} GROUP BY day ORDER BY day"
    )
}

#[derive(Debug, Serialize, Deserialize, clickhouse::Row, utoipa::ToSchema)]
pub struct UsageTotals {
    pub visits: u64,
    pub visitors: u64,
    pub page_views: u64,
}

#[derive(Debug, Serialize, Deserialize, clickhouse::Row, utoipa::ToSchema)]
pub struct UsageDay {
    pub day: String,
    pub visits: u64,
    pub visitors: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UsageSummaryResponse {
    pub since: String,
    pub until: String,
    pub totals: UsageTotals,
    pub by_day: Vec<UsageDay>,
    pub by_person: Vec<UsagePerson>,
    pub by_page: Vec<UsagePage>,
    pub by_event: Vec<UsageEvent>,
}
impl toolkit::api::api_dto::ResponseApiDto for UsageSummaryResponse {}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UsageConfigResponse {
    /// Whether this instance records usage at all.
    pub enabled: bool,
}
impl toolkit::api::api_dto::ResponseApiDto for UsageConfigResponse {}

pub async fn get_usage_config(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<impl IntoResponse, CanonicalError> {
    Ok(Json(UsageConfigResponse {
        enabled: state.config.usage.enabled,
    }))
}

pub async fn get_usage_summary(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Query(range): Query<UsageRangeQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let window = range.window()?;
    let tenant = ctx.subject_tenant_id().to_string();
    let since = window.since.to_string();
    let until = window.until.to_string();
    let bound = |sql: String| {
        state
            .ch
            .query(&sql)
            .bind(tenant.clone())
            .bind(since.clone())
            .bind(until.clone())
    };
    let visitors = format!("uniqExactIf(person_id, person_id != toUUID('{NIL_UUID}'))");

    let (totals, by_day, by_person, by_page, by_event) = tokio::try_join!(
        bound(totals_sql(&visitors)).fetch_one::<UsageTotals>(),
        bound(by_day_sql(&visitors)).fetch_all::<UsageDay>(),
        people_query(&state.ch, &tenant, &since, &until).fetch_all::<UsagePerson>(),
        bound(by_page_sql(&visitors)).fetch_all::<UsagePage>(),
        bound(actions_sql(&visitors)).fetch_all::<UsageEvent>(),
    )
    .map_err(read_error)?;

    Ok(Json(UsageSummaryResponse {
        since,
        until,
        totals,
        by_day,
        by_person,
        by_page,
        by_event,
    }))
}

#[cfg(test)]
mod tests;
