use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::error::UsageError;
use super::{AppState, is_admin_caller};

/// DDL owned by `scripts/migrations/20260816000000_usage-events.sql`; the
/// service holds INSERT and SELECT here, never CREATE.
const TABLE: &str = "product_usage.usage_events";

const MAX_RECORDS: usize = 200;

/// The two `LowCardinality` columns, where an unbounded value blows the dictionary.
const MAX_NAME: usize = 64;

const MAX_FIELD: usize = 128;

const MAX_PATH: usize = 512;

const BREAKDOWN_LIMIT: u32 = 200;

const PAGE_VIEW: &str = "page_view";

const SESSION_START: &str = "session_start";

const NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";

const DEFAULT_WINDOW_DAYS: i64 = 29;

const MAX_WINDOW_DAYS: i64 = 400;

const WINDOW: &str =
    "tenant_id = toUUID(?) AND toDate(ts) >= toDate(?) AND toDate(ts) <= toDate(?)";

/// A record carrying no session id stores `''`, and every such row across every
/// person and day is that same value — one phantom visit if counted naively.
const VISITS: &str = "uniqExactIf(session_id, session_id != '')";

/// SDK v2 body. Fields shared by every record are hoisted out of them into
/// `meta`, so a record carries only what differs.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UsageIngestRequest {
    #[serde(default)]
    pub meta: TelemetryRecord,
    #[serde(default)]
    pub records: Vec<TelemetryRecord>,
}
impl toolkit::api::api_dto::RequestApiDto for UsageIngestRequest {}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct TelemetryRecord {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub context_session_id: Option<String>,
    #[serde(default)]
    pub context_app_name: Option<String>,
    #[serde(default)]
    pub context_app_version: Option<String>,
    /// Epoch milliseconds on the browser's clock: when the event happened.
    #[serde(default)]
    pub time_triggered: Option<i64>,
    /// Epoch milliseconds on the same clock: when the batch was flushed.
    #[serde(default)]
    pub time_sent: Option<i64>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

pub async fn ingest_usage_events(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<UsageIngestRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    if !state.config.usage.enabled {
        return Ok(StatusCode::NO_CONTENT);
    }

    let tenant_id = ctx.subject_tenant_id();
    let person_id = ctx.subject_id();
    let arrival = Utc::now();

    let rows = recordable_rows(&req, tenant_id, person_id, arrival);

    if let Err(error) = insert_records(&state, &rows).await {
        // SAFETY: a write that fails forever reads as "nobody used the
        // product", so the swallow is logged even though one lost beacon
        // does not matter.
        tracing::warn!(error = %error, "usage event write failed");
    }

    Ok(StatusCode::NO_CONTENT)
}

fn recordable_rows(
    req: &UsageIngestRequest,
    tenant_id: Uuid,
    person_id: Uuid,
    arrival: DateTime<Utc>,
) -> Vec<UsageEventRow> {
    req.records
        .iter()
        .take(MAX_RECORDS)
        .map(|record| to_row(record, &req.meta, tenant_id, person_id, arrival))
        .filter(is_recordable)
        .collect()
}

/// INVARIANT: `event_id` is omitted so the table's DEFAULT applies.
#[derive(Debug, Serialize, clickhouse::Row)]
struct UsageEventRow {
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    ts: DateTime<Utc>,
    #[serde(with = "clickhouse::serde::uuid")]
    tenant_id: Uuid,
    #[serde(with = "clickhouse::serde::uuid")]
    person_id: Uuid,
    session_id: String,
    event_name: String,
    path: String,
    target: String,
    app_name: String,
    app_version: String,
}

/// How long a record may plausibly have waited in the browser's buffer. Past
/// this the correction cannot place it in the right day, which is the only
/// thing it exists to protect.
const MAX_BUFFERED_MS: i64 = 24 * 60 * 60 * 1000;

/// Both stamps come off the browser's clock, so their difference — how long the
/// record waited to be flushed — survives a clock that is hours out. Anchoring
/// that to the arrival instant keeps the timestamp ours while still separating
/// records the SDK sent in one beacon.
fn event_time(
    record: &TelemetryRecord,
    meta: &TelemetryRecord,
    arrival: DateTime<Utc>,
) -> DateTime<Utc> {
    let (Some(triggered), Some(sent)) = (
        record.time_triggered.or(meta.time_triggered),
        record.time_sent.or(meta.time_sent),
    ) else {
        return arrival;
    };
    match sent.checked_sub(triggered) {
        Some(buffered) if (0..=MAX_BUFFERED_MS).contains(&buffered) => {
            arrival - Duration::milliseconds(buffered)
        }
        _ => arrival,
    }
}

/// A field the SDK hoists into `meta` is cloned into every record of the batch,
/// so an unclipped one costs `MAX_RECORDS` times its own size.
fn shared(own: Option<&str>, meta: Option<&str>, max: usize) -> String {
    clip(own.or(meta).unwrap_or_default(), max)
}

fn to_row(
    record: &TelemetryRecord,
    meta: &TelemetryRecord,
    tenant_id: Uuid,
    person_id: Uuid,
    arrival: DateTime<Utc>,
) -> UsageEventRow {
    let data = record.data.as_ref().or(meta.data.as_ref());
    UsageEventRow {
        ts: event_time(record, meta, arrival),
        tenant_id,
        person_id,
        session_id: shared(
            record.context_session_id.as_deref(),
            meta.context_session_id.as_deref(),
            MAX_FIELD,
        ),
        event_name: shared(record.name.as_deref(), meta.name.as_deref(), MAX_NAME),
        path: clip(&data_field(data, "path"), MAX_PATH),
        target: clip(&data_field(data, "target"), MAX_PATH),
        app_name: shared(
            record.context_app_name.as_deref(),
            meta.context_app_name.as_deref(),
            MAX_NAME,
        ),
        app_version: shared(
            record.context_app_version.as_deref(),
            meta.context_app_version.as_deref(),
            MAX_FIELD,
        ),
    }
}

async fn insert_records(state: &AppState, rows: &[UsageEventRow]) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    let client = state.ch.inner().clone().with_setting("async_insert", "1");
    // Not `insert`: it escapes the name as one identifier, and `TABLE` is qualified.
    let mut insert = client.insert_unescaped::<UsageEventRow>(TABLE).await?;
    for row in rows {
        insert.write(row).await?;
    }
    insert.end().await?;
    Ok(())
}

/// The SDK stringifies each nested `data` value, so it arrives JSON-encoded.
fn data_field(data: Option<&serde_json::Value>, key: &str) -> String {
    let Some(serde_json::Value::Object(map)) = data else {
        return String::new();
    };
    match map.get(key) {
        Some(serde_json::Value::String(raw)) => {
            serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.clone())
        }
        _ => String::new(),
    }
}

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

fn by_page_sql(visitors: &str) -> String {
    format!(
        "SELECT path, count() AS views, {visitors} AS visitors \
         FROM {TABLE} WHERE {WINDOW} AND event_name = '{PAGE_VIEW}' AND path != '' \
         GROUP BY path ORDER BY views DESC LIMIT {BREAKDOWN_LIMIT}"
    )
}

/// The one read whose binds do not match the shared window: the identity join
/// scopes by tenant again, so the fourth value is bound here beside the `?`
/// that needs it rather than by the caller.
fn people_query(
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
fn people_sql() -> String {
    format!(
        "SELECT toString(u.person) AS person_id, \
         coalesce(p.display_name, '') AS display_name, \
         u.visits AS visits, u.page_views AS page_views, u.last_seen AS last_seen \
         FROM (\
           SELECT person_id AS person, {VISITS} AS visits, \
           countIf(event_name = '{PAGE_VIEW}') AS page_views, \
           toString(max(ts)) AS last_seen \
           FROM {TABLE} WHERE {WINDOW} AND person_id != toUUID('{NIL_UUID}') \
           GROUP BY person ORDER BY visits DESC, page_views DESC \
           LIMIT {BREAKDOWN_LIMIT}) AS u \
         LEFT JOIN (\
           SELECT person_id, \
           coalesce(\
             nullIf(argMaxIf(value_effective, (created_at, id), value_type = 'display_name'), ''), \
             nullIf(trimBoth(concat(\
               coalesce(argMaxIf(value_effective, (created_at, id), value_type = 'first_name'), ''), \
               ' ', \
               coalesce(argMaxIf(value_effective, (created_at, id), value_type = 'last_name'), '') \
             )), '') \
           ) AS display_name \
           FROM identity.identity_persons \
           WHERE value_type IN ('display_name', 'first_name', 'last_name') \
           AND insight_tenant_id = toUUID(?) \
           GROUP BY person_id) AS p ON p.person_id = u.person \
         ORDER BY u.visits DESC, u.page_views DESC"
    )
}

/// Excludes the two events already counted as visits.
fn actions_sql(visitors: &str) -> String {
    format!(
        "SELECT event_name, target, count() AS opens, {visitors} AS people \
         FROM {TABLE} WHERE {WINDOW} \
         AND event_name NOT IN ('{PAGE_VIEW}', '{SESSION_START}') \
         GROUP BY event_name, target ORDER BY opens DESC LIMIT {BREAKDOWN_LIMIT}"
    )
}

/// The SDK emits its own page view naming the location `url`; the app emits one
/// with `path`. Keeping both counts every page twice.
fn is_recordable(row: &UsageEventRow) -> bool {
    row.event_name != PAGE_VIEW || !row.path.is_empty()
}

fn clip(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UsageRangeQuery {
    /// Inclusive `YYYY-MM-DD` lower bound. Defaults to 30 days back.
    pub since: Option<String>,
    /// Inclusive `YYYY-MM-DD` upper bound. Defaults to today.
    pub until: Option<String>,
}

struct Window {
    since: NaiveDate,
    until: NaiveDate,
}

impl UsageRangeQuery {
    fn window(&self) -> Result<Window, CanonicalError> {
        let until =
            parse_day("until", self.until.as_deref())?.unwrap_or_else(|| Utc::now().date_naive());
        let since = parse_day("since", self.since.as_deref())?
            .unwrap_or_else(|| until - Duration::days(DEFAULT_WINDOW_DAYS));
        if since > until {
            return Err(range_violation("since", "since must not be after until"));
        }
        if (until - since).num_days() >= MAX_WINDOW_DAYS {
            return Err(range_violation(
                "since",
                "the window must not exceed 400 days",
            ));
        }
        Ok(Window { since, until })
    }
}

fn range_violation(field: &str, description: &str) -> CanonicalError {
    UsageError::invalid_argument()
        .with_field_violation(field, description, "INVALID")
        .create()
}

fn parse_day(field: &str, value: Option<&str>) -> Result<Option<NaiveDate>, CanonicalError> {
    value
        .map(|day| {
            NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .map_err(|_| range_violation(field, "date must use YYYY-MM-DD"))
        })
        .transpose()
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

#[derive(Debug, Serialize, Deserialize, clickhouse::Row, utoipa::ToSchema)]
pub struct UsagePerson {
    pub person_id: String,
    /// Empty when the visitor has not been mirrored into the identity rows yet.
    pub display_name: String,
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
    require_admin(&state, &headers).await?;

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

#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn read_error(error: clickhouse::error::Error) -> CanonicalError {
    tracing::error!(error = %error, "usage summary query failed");
    CanonicalError::internal("failed to read usage").create()
}

async fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), CanonicalError> {
    if is_admin_caller(state, headers).await? {
        return Ok(());
    }

    Err(UsageError::permission_denied()
        .with_reason("admin role required for this operation")
        .create())
}

#[cfg(test)]
mod tests;
