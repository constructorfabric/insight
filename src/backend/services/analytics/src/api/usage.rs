//! Usage monitoring — `/v1/usage/*`.
//!
//! Adoption events emitted by the SPA telemetry SDK, and the admin read model
//! over them: who opened the product, how many visits per day, which pages.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::error::UsageError;
use super::{AppState, forwarded_authorization};

/// The writable namespace of the presentation query path: `presentation_ro`
/// grants SELECT/INSERT/CREATE there and read-only everywhere else, so a
/// service-owned table has exactly one place it can live.
const TABLE: &str = "presentation.usage_events";

/// Bound on one beacon payload. The SDK batches on a flush timer, so a larger
/// body is a client that is not the SDK.
const MAX_RECORDS: usize = 200;

/// Rows returned per breakdown. The page shows a ranked table, not an export.
const BREAKDOWN_LIMIT: u32 = 200;

/// The event the SPA emits on every resolved route.
const PAGE_VIEW: &str = "page_view";

/// The SDK's own lifecycle event, emitted once per session.
const SESSION_START: &str = "session_start";

const NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";

pub const CREATE_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS presentation.usage_events (
    event_id    UUID,
    ts          DateTime64(3, 'UTC'),
    tenant_id   UUID,
    person_id   UUID,
    session_id  String,
    event_name  LowCardinality(String),
    path        String,
    target      String,
    app_name    LowCardinality(String),
    app_version String
) ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (tenant_id, ts, event_id)";

/// Create the events table if it is absent. Idempotent, and safe to run on
/// every boot.
///
/// # Errors
///
/// Returns an error when ClickHouse rejects the statement or is unreachable.
pub async fn ensure_schema(ch: &insight_clickhouse::Client) -> anyhow::Result<()> {
    ch.query(CREATE_TABLE_SQL).execute().await?;
    Ok(())
}

// ── Ingest ──────────────────────────────────────────────────

/// The SDK's transport envelope: a Kafka REST Proxy body, accepted as it
/// stands so the published SDK needs no fork.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UsageIngestRequest {
    #[serde(default)]
    pub records: Vec<UsageIngestEnvelope>,
}
impl toolkit::api::api_dto::RequestApiDto for UsageIngestRequest {}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UsageIngestEnvelope {
    pub value: TelemetryRecord,
}

/// The subset of the SDK's record this service stores. Everything else it
/// sends — device, OS, viewport, locale — is dropped at ingest.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TelemetryRecord {
    pub name: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub time_triggered: Option<i64>,
    #[serde(default)]
    pub context_session_id: Option<String>,
    #[serde(default)]
    pub context_app_name: Option<String>,
    #[serde(default)]
    pub context_app_version: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// `POST /v1/usage/events` — the SPA's beacon.
///
/// Answers 204 whatever happens: the caller is a fire-and-forget beacon that
/// cannot act on an error, and a tracking failure must never surface in the
/// product. Identity comes from the gateway JWT; the body's own identity
/// fields are ignored.
///
/// # Errors
///
/// Never — the signature keeps the handler shape uniform with its neighbours.
pub async fn ingest_usage_events(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<UsageIngestRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    if !state.config.usage.enabled || req.records.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }

    let tenant_id = ctx.subject_tenant_id();
    let person_id = ctx.subject_id();

    let records = &req.records[..req.records.len().min(MAX_RECORDS)];

    if let Err(error) = insert_records(&state, records, tenant_id, person_id).await {
        // SAFETY: a write that fails forever reads as "nobody used the
        // product", so the swallow is logged even though one lost beacon
        // does not matter.
        tracing::warn!(error = %error, "usage event write failed");
    }

    Ok(StatusCode::NO_CONTENT)
}

/// One row's worth of placeholders. Values are bound, never interpolated: the
/// SDK's payload is caller-controlled and reaches ClickHouse verbatim.
const ROW_PLACEHOLDERS: &str = "(toUUID(?), fromUnixTimestamp64Milli(toInt64(?)), \
     toUUID(?), toUUID(?), ?, ?, ?, ?, ?, ?)";

fn insert_sql(row_count: usize) -> String {
    let rows = vec![ROW_PLACEHOLDERS; row_count].join(", ");
    // Beacons arrive one browser tab at a time; server-side batching keeps a
    // burst of them from becoming a part each.
    format!(
        "INSERT INTO {TABLE} \
         (event_id, ts, tenant_id, person_id, session_id, event_name, path, target, \
          app_name, app_version) \
         SETTINGS async_insert = 1 VALUES {rows}"
    )
}

async fn insert_records(
    state: &AppState,
    records: &[UsageIngestEnvelope],
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<()> {
    if records.is_empty() {
        return Ok(());
    }

    let mut query = state.ch.query(&insert_sql(records.len()));
    for envelope in records {
        let record = &envelope.value;
        let event_id = record
            .id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok())
            .unwrap_or_else(Uuid::now_v7);
        let millis = record
            .time_triggered
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

        query = query
            .bind(event_id.to_string())
            .bind(millis)
            .bind(tenant_id.to_string())
            .bind(person_id.to_string())
            .bind(clip(record.context_session_id.as_deref().unwrap_or_default(), 64))
            .bind(clip(&record.name, 64))
            .bind(clip(&page_path(record.data.as_ref()), 512))
            .bind(clip(&data_field(record.data.as_ref(), "target"), 256))
            .bind(clip(record.context_app_name.as_deref().unwrap_or_default(), 64))
            .bind(clip(record.context_app_version.as_deref().unwrap_or_default(), 32));
    }

    query.execute().await?;
    Ok(())
}

/// The SDK stringifies each nested `data` field before sending, so a value
/// arrives either as a plain string or as a JSON-encoded one.
fn data_field(data: Option<&serde_json::Value>, key: &str) -> String {
    let Some(value) = data else {
        return String::new();
    };
    let object = match value {
        serde_json::Value::String(raw) => serde_json::from_str::<serde_json::Value>(raw).ok(),
        other => Some(other.clone()),
    };
    let Some(serde_json::Value::Object(map)) = object else {
        return String::new();
    };
    match map.get(key) {
        Some(serde_json::Value::String(raw)) => {
            serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.clone())
        }
        _ => String::new(),
    }
}

/// Where a page view happened. The SDK's own page view names it `url`; the
/// one the app emits names it `path`.
/// Deliberate actions only: a page view has its own breakdown, and
/// `session_start` is the SDK announcing a session rather than anyone doing
/// something. Both are already counted as visits.
fn actions_sql(window: &str, visitors: &str) -> String {
    format!(
        "SELECT event_name, target, count() AS opens, {visitors} AS people \
         FROM {TABLE} WHERE {window} \
         AND event_name NOT IN ('{PAGE_VIEW}', '{SESSION_START}') \
         GROUP BY event_name, target ORDER BY opens DESC LIMIT {BREAKDOWN_LIMIT}"
    )
}

fn page_path(data: Option<&serde_json::Value>) -> String {
    let path = data_field(data, "path");
    if path.is_empty() {
        return data_field(data, "url");
    }
    path
}

fn clip(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

// ── Read model ──────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UsageRangeQuery {
    /// Inclusive `YYYY-MM-DD` lower bound. Defaults to 30 days back.
    pub since: Option<String>,
    /// Inclusive `YYYY-MM-DD` upper bound. Defaults to today.
    pub until: Option<String>,
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

/// `GET /v1/usage/config` — whether the instance collects usage.
///
/// Any signed-in caller: the SPA reads it to decide whether to start the SDK,
/// and it carries no usage data.
///
/// # Errors
///
/// Never — the signature keeps the handler shape uniform with its neighbours.
pub async fn get_usage_config(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<impl IntoResponse, CanonicalError> {
    Ok(Json(UsageConfigResponse {
        enabled: state.config.usage.enabled,
    }))
}

/// `GET /v1/usage/summary` — the admin read model.
///
/// # Errors
///
/// 403 when the caller holds no admin role, 500 when ClickHouse fails.
pub async fn get_usage_summary(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Query(range): Query<UsageRangeQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers).await?;

    let tenant_id = ctx.subject_tenant_id();
    let since = day_bound(range.since.as_deref(), "toDate(now()) - 29");
    let until = day_bound(range.until.as_deref(), "toDate(now())");
    let window =
        format!("tenant_id = toUUID('{tenant_id}') AND toDate(ts) >= {since} AND toDate(ts) <= {until}");
    let visitors = format!("uniqExactIf(person_id, person_id != toUUID('{NIL_UUID}'))");

    let totals = state
        .ch
        .query(&format!(
            "SELECT uniqExact(session_id) AS visits, {visitors} AS visitors, \
             countIf(event_name = '{PAGE_VIEW}') AS page_views \
             FROM {TABLE} WHERE {window}"
        ))
        .fetch_one::<UsageTotals>()
        .await
        .map_err(read_error)?;

    let by_day = state
        .ch
        .query(&format!(
            "SELECT toString(toDate(ts)) AS day, uniqExact(session_id) AS visits, \
             {visitors} AS visitors \
             FROM {TABLE} WHERE {window} GROUP BY day ORDER BY day"
        ))
        .fetch_all::<UsageDay>()
        .await
        .map_err(read_error)?;

    let by_person = state
        .ch
        .query(&format!(
            // The id is stringified in an outer select: an alias that shadows
            // `person_id` makes every other reference to it a String, and the
            // UUID comparisons in the same query then have no common type.
            "SELECT toString(person) AS person_id, visits, page_views, last_seen FROM (\
               SELECT person_id AS person, uniqExact(session_id) AS visits, \
               countIf(event_name = '{PAGE_VIEW}') AS page_views, \
               toString(max(ts)) AS last_seen \
               FROM {TABLE} WHERE {window} AND person_id != toUUID('{NIL_UUID}') \
               GROUP BY person ORDER BY visits DESC, page_views DESC \
               LIMIT {BREAKDOWN_LIMIT})"
        ))
        .fetch_all::<UsagePerson>()
        .await
        .map_err(read_error)?;

    let by_page = state
        .ch
        .query(&format!(
            "SELECT path, count() AS views, {visitors} AS visitors \
             FROM {TABLE} WHERE {window} AND event_name = '{PAGE_VIEW}' AND path != '' \
             GROUP BY path ORDER BY views DESC LIMIT {BREAKDOWN_LIMIT}"
        ))
        .fetch_all::<UsagePage>()
        .await
        .map_err(read_error)?;

    let by_event = state
        .ch
        .query(&actions_sql(&window, &visitors))
        .fetch_all::<UsageEvent>()
        .await
        .map_err(read_error)?;

    Ok(Json(UsageSummaryResponse {
        since: range.since.unwrap_or_default(),
        until: range.until.unwrap_or_default(),
        totals,
        by_day,
        by_person,
        by_page,
        by_event,
    }))
}

/// A caller-supplied day, or the SQL expression that stands in for it. Anything
/// that is not a plain `YYYY-MM-DD` falls back to the default rather than
/// reaching the query.
fn day_bound(value: Option<&str>, default_expr: &str) -> String {
    match value {
        Some(day) if is_iso_day(day) => format!("toDate('{day}')"),
        _ => default_expr.to_owned(),
    }
}

fn is_iso_day(value: &str) -> bool {
    value.len() == 10
        && value
            .chars()
            .enumerate()
            .all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() })
}

fn read_error(error: clickhouse::error::Error) -> CanonicalError {
    tracing::error!(error = %error, "usage summary query failed");
    CanonicalError::internal("failed to read usage").create()
}

/// The usage surface is admin-only, and the check is server-side: the nav flag
/// the SPA reads is a courtesy, not the boundary.
async fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), CanonicalError> {
    let is_admin = state
        .identity
        .is_admin(forwarded_authorization(headers))
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "admin role check failed");
            CanonicalError::internal("failed to verify caller permissions").create()
        })?;

    if is_admin {
        return Ok(());
    }
    Err(UsageError::permission_denied()
        .with_reason("admin role required for this operation")
        .create())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_survives_the_sdk_field_stringification() {
        let stringified = serde_json::json!({ "path": "\"/people\"" });
        assert_eq!(data_field(Some(&stringified), "path"), "/people");

        let plain = serde_json::json!({ "target": "pr_cycle_time" });
        assert_eq!(data_field(Some(&plain), "target"), "pr_cycle_time");
    }

    #[test]
    fn the_sdks_own_page_view_reports_the_url_it_carries() {
        // The SDK emits its own page_view with `url`; ours carries `path`.
        let from_sdk = serde_json::json!({ "url": "\"/portal\"" });
        assert_eq!(page_path(Some(&from_sdk)), "/portal");

        let from_app = serde_json::json!({ "path": "\"/portal/people\"" });
        assert_eq!(page_path(Some(&from_app)), "/portal/people");
    }

    #[test]
    fn a_record_without_data_has_no_path() {
        assert_eq!(data_field(None, "path"), "");
        assert_eq!(data_field(Some(&serde_json::json!({})), "path"), "");
    }

    #[test]
    fn event_values_are_bound_never_interpolated() {
        let sql = insert_sql(2);
        assert_eq!(sql.matches('?').count(), 20, "ten placeholders per row: {sql}");
        assert!(sql.contains("async_insert = 1"), "batched server-side: {sql}");
    }

    #[test]
    fn the_actions_breakdown_leaves_out_what_nobody_did() {
        // `page_view` is its own table and `session_start` is the SDK's
        // lifecycle event — both are already counted as visits.
        let sql = actions_sql("1", "2");
        assert!(!sql.contains("event_name = 'page_view'"), "{sql}");
        for lifecycle in ["'page_view'", "'session_start'"] {
            assert!(
                sql.contains(&format!("event_name != {lifecycle}"))
                    || sql.contains(&format!("NOT IN ('page_view', 'session_start')")),
                "excludes {lifecycle}: {sql}"
            );
        }
    }

    #[test]
    fn only_a_calendar_day_reaches_the_query() {
        assert_eq!(day_bound(Some("2026-01-31"), "fallback"), "toDate('2026-01-31')");
        assert_eq!(day_bound(Some("2026-01-31' OR 1=1"), "fallback"), "fallback");
        assert_eq!(day_bound(None, "fallback"), "fallback");
    }
}
