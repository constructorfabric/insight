use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::date_window::{self, WINDOW};
use super::error::FeedbackError;
use super::person_names::NAMED_PERSONS;
use super::{AppState, is_admin_caller};

/// DDL owned by `scripts/migrations/20260821000000_product-feedback.sql`; the
/// service holds INSERT and SELECT here, never CREATE.
const TABLE: &str = "product_usage.feedback";

const MAX_MESSAGE: usize = 4000;

const MAX_PATH: usize = 512;

const MAX_FIELD: usize = 128;

/// The `LowCardinality` column, where an unbounded value blows the dictionary.
const MAX_NAME: usize = 64;

const LIST_LIMIT: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackCategory {
    Bug,
    Idea,
    Confusing,
    Other,
}

impl FeedbackCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::Idea => "idea",
            Self::Confusing => "confusing",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct FeedbackRequest {
    pub category: FeedbackCategory,
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

/// INVARIANT: `feedback_id` is omitted so the table's DEFAULT applies.
#[derive(Debug, Serialize, clickhouse::Row)]
struct FeedbackRow {
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    ts: DateTime<Utc>,
    #[serde(with = "clickhouse::serde::uuid")]
    tenant_id: Uuid,
    #[serde(with = "clickhouse::serde::uuid")]
    person_id: Uuid,
    category: String,
    message: String,
    path: String,
    app_name: String,
    app_version: String,
}

#[derive(Debug, Serialize, Deserialize, clickhouse::Row, utoipa::ToSchema)]
pub struct FeedbackEntry {
    pub feedback_id: String,
    pub ts: String,
    pub person_id: String,
    /// Empty when the sender has not been mirrored into the identity rows yet.
    pub display_name: String,
    /// The account handle, empty when no identity row carries one.
    pub username: String,
    pub category: String,
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

    insert_feedback(&state, &row).await.map_err(|error| {
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
    let tenant = ctx.subject_tenant_id().to_string();
    let since = window.since.to_string();
    let until = window.until.to_string();

    let items = state
        .ch
        .query(&list_sql())
        .bind(tenant.clone())
        .bind(since.clone())
        .bind(until.clone())
        .bind(tenant)
        .fetch_all::<FeedbackEntry>()
        .await
        .map_err(read_error)?;

    Ok(Json(FeedbackListResponse {
        since,
        until,
        items,
    }))
}

fn to_row(
    req: &FeedbackRequest,
    tenant_id: Uuid,
    person_id: Uuid,
    now: DateTime<Utc>,
) -> Result<FeedbackRow, CanonicalError> {
    let message = req.message.trim();
    if message.is_empty() {
        return Err(violation("message", "message must not be empty"));
    }

    Ok(FeedbackRow {
        ts: now,
        tenant_id,
        person_id,
        category: req.category.as_str().to_owned(),
        message: clip(message, MAX_MESSAGE),
        path: clip(&req.path, MAX_PATH),
        app_name: clip(&req.app_name, MAX_NAME),
        app_version: clip(&req.app_version, MAX_FIELD),
    })
}

async fn insert_feedback(state: &AppState, row: &FeedbackRow) -> anyhow::Result<()> {
    // Not `insert`: it escapes the name as one identifier, and `TABLE` is qualified.
    let mut insert = state
        .ch
        .inner()
        .insert_unescaped::<FeedbackRow>(TABLE)
        .await?;
    insert.write(row).await?;
    insert.end().await?;
    Ok(())
}

fn list_sql() -> String {
    format!(
        "SELECT toString(f.feedback_id) AS feedback_id, toString(f.ts) AS ts, \
         toString(f.person_id) AS person_id, \
         coalesce(p.display_name, '') AS display_name, \
         coalesce(p.username, '') AS username, \
         f.category AS category, f.message AS message, f.path AS path \
         FROM {TABLE} AS f \
         LEFT JOIN {NAMED_PERSONS} AS p ON p.person_id = f.person_id \
         WHERE {WINDOW} \
         ORDER BY f.ts DESC LIMIT {LIST_LIMIT}"
    )
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
fn read_error(error: clickhouse::error::Error) -> CanonicalError {
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
