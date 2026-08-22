use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect, Set};
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::error::FeedbackError;
use super::person_names::{self, PersonName};
use super::{ADMIN_ONLY, AppState, clip, require_admin};
use crate::domain::date_window::{self, WindowError};
use crate::infra::db::entities::feedback;
use crate::migration::feedback_schema;

/// The newest submissions in the window. A window holding more is cut without
/// saying so — narrowing the period is the only way to reach what falls past it.
const LIST_LIMIT: u64 = 200;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct FeedbackRequest {
    pub message: String,
    /// The screen the sender was on. Empty when the SPA cannot name one.
    #[serde(default)]
    pub path: String,
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
    require_admin(&state, &headers, admin_only).await?;

    let window = date_window::parse_window(range.since.as_deref(), range.until.as_deref())
        .map_err(refused_window)?;
    let tenant = ctx.subject_tenant_id();

    let rows = feedback::Entity::find()
        .filter(feedback::Column::InsightTenantId.eq(tenant))
        .filter(feedback::Column::CreatedAt.gte(day_start(window.since)))
        .filter(feedback::Column::CreatedAt.lt(day_after(window.until)))
        .order_by(feedback::Column::CreatedAt, Order::Desc)
        .limit(LIST_LIMIT)
        .all(&state.db)
        .await
        .map_err(read_error)?;

    let names = person_names::lookup(&state.ch, tenant, &senders(&rows)).await;

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
        message: Set(clip(message, feedback_schema::max_message())),
        path: Set(clip(&req.path, feedback_schema::max_path())),
        created_at: Set(now),
    })
}

/// Who to ask the identity mirror about: one id per person on the page, not
/// one per row.
fn senders(rows: &[feedback::Model]) -> Vec<Uuid> {
    rows.iter()
        .map(|row| row.person_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn entry(row: feedback::Model, names: &HashMap<Uuid, PersonName>) -> FeedbackEntry {
    let (display_name, username) = names
        .get(&row.person_id)
        .map(|sender| (sender.display_name.clone(), sender.username.clone()))
        .unwrap_or_default();

    FeedbackEntry {
        feedback_id: row.id.to_string(),
        ts: row.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        person_id: row.person_id.to_string(),
        display_name,
        username,
        message: row.message,
        path: row.path,
    }
}

fn day_start(day: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&day.and_time(NaiveTime::MIN))
}

/// The window names whole UTC days, so the upper bound is the midnight AFTER
/// `until` — anything narrower drops part of that day.
fn day_after(day: NaiveDate) -> DateTime<Utc> {
    day_start(day) + Duration::days(1)
}

fn admin_only() -> CanonicalError {
    FeedbackError::permission_denied()
        .with_reason(ADMIN_ONLY)
        .create()
}

fn refused_window(error: WindowError) -> CanonicalError {
    violation(error.field(), &error.description())
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

#[cfg(test)]
mod tests;
