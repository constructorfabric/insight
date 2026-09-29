use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::super::{AppState, require_admin};
use super::{
    PAGE_VIEW, TABLE, UsageRangeQuery, VISITS, WINDOW, WindowBinds, admin_only, read_error,
    visitors,
};

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
    let binds = WindowBinds::new(&ctx, &window);
    let visitors = visitors();

    let (totals, by_day) = tokio::try_join!(
        binds
            .query(&state.ch, &totals_sql(&visitors))
            .fetch_one::<UsageTotals>(),
        binds
            .query(&state.ch, &by_day_sql(&visitors))
            .fetch_all::<UsageDay>(),
    )
    .map_err(read_error)?;

    Ok(Json(UsageSummaryResponse {
        since: binds.since,
        until: binds.until,
        totals,
        by_day,
    }))
}

#[cfg(test)]
mod tests;
