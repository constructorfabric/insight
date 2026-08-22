use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect, Set};
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::date_window;
use super::error::FeedbackError;
use super::person_names::NAMED_PERSONS;
use super::{AppState, is_admin_caller};
use crate::infra::db::entities::feedback;

const MAX_MESSAGE: usize = 4000;

const MAX_PATH: usize = 512;

const MAX_FIELD: usize = 128;

const MAX_NAME: usize = 64;

const LIST_LIMIT: u64 = 200;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct FeedbackRequest {
    pub message: String,
    /// The screen the sender was on. Empty when the SPA cannot name one.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub app_name: String,
    #[serde(default)]
    pub app_version: String,
}
impl toolkit::api::api_dto::RequestApiDto for FeedbackRequest {}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct FeedbackEntry {
    pub feedback_id: String,
    pub ts: String,
    pub person_id: String,
    /// Empty when the sender has not been mirrored into the identity rows yet.
    pub display_name: String,
    /// The account handle, empty when no identity row carries one.
    pub username: String,
    pub message: String,
    pub path: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct FeedbackListResponse {
    pub since: String,
    pub until: String,
    pub items: Vec<FeedbackEntry>,
}
impl toolkit::api::api_dto::ResponseApiDto for FeedbackListResponse {}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct FeedbackRangeQuery {
    /// Inclusive `YYYY-MM-DD` lower bound. Defaults to 30 days back.
    pub since: Option<String>,
    /// Inclusive `YYYY-MM-DD` upper bound. Defaults to today.
    pub until: Option<String>,
}

pub async fn submit_feedback(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<FeedbackRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let row = to_row(&req, ctx.subject_tenant_id(), ctx.subject_id(), Utc::now())?;

    feedback::Entity::insert(row)
        .exec(&state.db)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "feedback write failed");
            CanonicalError::internal("failed to record feedback").create()
        })?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_feedback(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Query(range): Query<FeedbackRangeQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers).await?;

    let window =
        date_window::parse_window(range.since.as_deref(), range.until.as_deref(), violation)?;
    let tenant = ctx.subject_tenant_id();

    let rows = feedback::Entity::find()
        .filter(feedback::Column::InsightTenantId.eq(tenant))
        .filter(feedback::Column::CreatedAt.gte(day_start(window.since)))
        .filter(feedback::Column::CreatedAt.lte(day_end(window.until)))
        .order_by(feedback::Column::CreatedAt, Order::Desc)
        .limit(LIST_LIMIT)
        .all(&state.db)
        .await
        .map_err(read_error)?;

    let names = person_names(&state, tenant, &rows).await;

    Ok(Json(FeedbackListResponse {
        since: window.since.to_string(),
        until: window.until.to_string(),
        items: rows.into_iter().map(|row| entry(row, &names)).collect(),
    }))
}

fn to_row(
    req: &FeedbackRequest,
    tenant_id: Uuid,
    person_id: Uuid,
    now: DateTime<Utc>,
) -> Result<feedback::ActiveModel, CanonicalError> {
    let message = req.message.trim();
    if message.is_empty() {
        return Err(violation("message", "message must not be empty"));
    }

    Ok(feedback::ActiveModel {
        id: Set(Uuid::now_v7()),
        insight_tenant_id: Set(tenant_id),
        person_id: Set(person_id),
        message: Set(clip(message, MAX_MESSAGE)),
        path: Set(clip(&req.path, MAX_PATH)),
        app_name: Set(clip(&req.app_name, MAX_NAME)),
        app_version: Set(clip(&req.app_version, MAX_FIELD)),
        created_at: Set(now),
    })
}

fn entry(row: feedback::Model, names: &HashMap<String, PersonName>) -> FeedbackEntry {
    let person_id = row.person_id.to_string();
    let sender = names.get(&person_id);

    FeedbackEntry {
        feedback_id: row.id.to_string(),
        ts: row.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        display_name: sender.map(|n| n.display_name.clone()).unwrap_or_default(),
        username: sender.map(|n| n.username.clone()).unwrap_or_default(),
        person_id,
        message: row.message,
        path: row.path,
    }
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct PersonName {
    person_id: String,
    display_name: String,
    username: String,
}

/// The rows are service-DB content and the names are in the identity mirror, so
/// naming a sender is a second read rather than a join, asking only for the
/// people on this page.
///
/// A failed lookup leaves them unnamed rather than failing the listing: the
/// feedback itself is what is being read.
async fn person_names(
    state: &AppState,
    tenant: Uuid,
    rows: &[feedback::Model],
) -> HashMap<String, PersonName> {
    let ids: Vec<String> = rows
        .iter()
        .map(|row| row.person_id.to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    if ids.is_empty() {
        return HashMap::new();
    }

    let sql = format!(
        "SELECT toString(p.person_id) AS person_id, \
         coalesce(p.display_name, '') AS display_name, \
         coalesce(p.username, '') AS username \
         FROM {NAMED_PERSONS} AS p WHERE toString(p.person_id) IN ?"
    );

    match state
        .ch
        .query(&sql)
        .bind(tenant.to_string())
        .bind(ids)
        .fetch_all::<PersonName>()
        .await
    {
        Ok(found) => found
            .into_iter()
            .map(|name| (name.person_id.clone(), name))
            .collect(),
        Err(error) => {
            tracing::warn!(error = %error, "naming feedback senders failed");
            HashMap::new()
        }
    }
}

fn day_start(day: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&day.and_time(NaiveTime::MIN))
}

/// INVARIANT: the window is a pair of whole UTC days, so the upper bound has to
/// reach the end of `until` — comparing against its midnight drops that day.
fn day_end(day: NaiveDate) -> DateTime<Utc> {
    let last = NaiveTime::from_hms_opt(23, 59, 59).unwrap_or(NaiveTime::MIN);
    Utc.from_utc_datetime(&day.and_time(last))
}

fn clip(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn violation(field: &str, description: &str) -> CanonicalError {
    FeedbackError::invalid_argument()
        .with_field_violation(field, description, "INVALID")
        .create()
}

#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn read_error(error: sea_orm::DbErr) -> CanonicalError {
    tracing::error!(error = %error, "feedback listing query failed");
    CanonicalError::internal("failed to read feedback").create()
}

async fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), CanonicalError> {
    if is_admin_caller(state, headers).await? {
        return Ok(());
    }
    Err(FeedbackError::permission_denied()
        .with_reason("admin role required for this operation")
        .create())
}

#[cfg(test)]
mod tests;
