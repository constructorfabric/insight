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
use chrono::{Duration, NaiveDate, Utc};
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

/// How far back a summary looks when the caller names no window.
const DEFAULT_WINDOW_DAYS: i64 = 29;

/// Every value a caller controls is bound; the constants above are not.
const WINDOW: &str = "tenant_id = toUUID(?) AND toDate(ts) >= toDate(?) AND toDate(ts) <= toDate(?)";

const CREATE_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS presentation.usage_events (
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
            .bind(clip(&normalize_path(&page_path(record.data.as_ref())), 512))
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

/// Who opened the product, named.
///
/// The names come from the identity rows mirrored into ClickHouse rather than
/// from a per-caller profile lookup: that lookup answers only for people inside
/// the caller's visible set, and "who uses the product" is an org-wide question
/// on an admin-only surface. A visitor identity has not mirrored yet keeps an
/// empty name and is shown by id.
fn people_sql() -> String {
    format!(
        "SELECT toString(u.person) AS person_id, \
         coalesce(p.display_name, '') AS display_name, \
         u.visits AS visits, u.page_views AS page_views, u.last_seen AS last_seen \
         FROM (\
           SELECT person_id AS person, uniqExact(session_id) AS visits, \
           countIf(event_name = '{PAGE_VIEW}') AS page_views, \
           toString(max(ts)) AS last_seen \
           FROM {TABLE} WHERE {WINDOW} AND person_id != toUUID('{NIL_UUID}') \
           GROUP BY person ORDER BY visits DESC, page_views DESC \
           LIMIT {BREAKDOWN_LIMIT}) AS u \
         LEFT JOIN (\
           SELECT person_id, any(value_effective) AS display_name \
           FROM identity.identity_persons \
           WHERE value_type = 'display_name' AND insight_tenant_id = toUUID(?) \
           GROUP BY person_id) AS p ON p.person_id = u.person \
         ORDER BY u.visits DESC, u.page_views DESC"
    )
}

/// Deliberate actions only: a page view has its own breakdown, and
/// `session_start` is the SDK announcing a session rather than anyone doing
/// something. Both are already counted as visits.
fn actions_sql(visitors: &str) -> String {
    format!(
        "SELECT event_name, target, count() AS opens, {visitors} AS people \
         FROM {TABLE} WHERE {WINDOW} \
         AND event_name NOT IN ('{PAGE_VIEW}', '{SESSION_START}') \
         GROUP BY event_name, target ORDER BY opens DESC LIMIT {BREAKDOWN_LIMIT}"
    )
}

/// Where a page view happened. The SDK's own page view names it `url`; the one
/// the app emits names it `path`.
fn page_path(data: Option<&serde_json::Value>) -> String {
    let path = data_field(data, "path");
    if path.is_empty() {
        return data_field(data, "url");
    }
    path
}

/// A path names a screen, never a person. `/ic/<uuid>/personal` is one screen
/// whoever it is about, and storing the id would turn adoption counting into a
/// record of who read whose profile.
fn normalize_path(path: &str) -> String {
    path.split('/')
        .map(|segment| if is_identifier(segment) { ":id" } else { segment })
        .collect::<Vec<_>>()
        .join("/")
}

/// A UUID, or any other long opaque segment a route carries as an id.
fn is_identifier(segment: &str) -> bool {
    let uuid_shaped = segment.len() == 36
        && segment
            .split('-')
            .map(str::len)
            .eq([8, 4, 4, 4, 12])
        && segment.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    let all_digits = segment.len() >= 6 && segment.chars().all(|c| c.is_ascii_digit());
    uuid_shaped || all_digits
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

/// The window a summary covers, as calendar days.
struct Window {
    since: NaiveDate,
    until: NaiveDate,
}

impl UsageRangeQuery {
    fn window(&self) -> Result<Window, CanonicalError> {
        let until = parse_day("until", self.until.as_deref())?
            .unwrap_or_else(|| Utc::now().date_naive());
        let since = parse_day("since", self.since.as_deref())?
            .unwrap_or_else(|| until - Duration::days(DEFAULT_WINDOW_DAYS));
        Ok(Window { since, until })
    }
}

fn parse_day(field: &str, value: Option<&str>) -> Result<Option<NaiveDate>, CanonicalError> {
    value
        .map(|day| {
            NaiveDate::parse_from_str(day, "%Y-%m-%d").map_err(|_| {
                UsageError::invalid_argument()
                    .with_field_violation(field, "date must use YYYY-MM-DD", "INVALID")
                    .create()
            })
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

/// `GET /v1/usage/config` — whether the instance collects usage. Any signed-in
/// caller: the SPA reads it to decide whether to start the SDK.
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

    // Five independent scans of one window; the handler waits for the slowest,
    // not for their sum.
    let (totals, by_day, by_person, by_page, by_event) = tokio::try_join!(
        bound(format!(
            "SELECT uniqExact(session_id) AS visits, {visitors} AS visitors, \
             countIf(event_name = '{PAGE_VIEW}') AS page_views \
             FROM {TABLE} WHERE {WINDOW}"
        ))
        .fetch_one::<UsageTotals>(),
        bound(format!(
            "SELECT toString(toDate(ts)) AS day, uniqExact(session_id) AS visits, \
             {visitors} AS visitors \
             FROM {TABLE} WHERE {WINDOW} GROUP BY day ORDER BY day"
        ))
        .fetch_all::<UsageDay>(),
        bound(people_sql())
            .bind(tenant.clone())
            .fetch_all::<UsagePerson>(),
        bound(format!(
            "SELECT path, count() AS views, {visitors} AS visitors \
             FROM {TABLE} WHERE {WINDOW} AND event_name = '{PAGE_VIEW}' AND path != '' \
             GROUP BY path ORDER BY views DESC LIMIT {BREAKDOWN_LIMIT}"
        ))
        .fetch_all::<UsagePage>(),
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

/// The usage surface is admin-only, and the check is server-side: the nav flag
/// the SPA reads is a courtesy, not the boundary.
async fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), CanonicalError> {
    if !state.identity.is_configured() {
        tracing::error!("identity service is not configured; admin access cannot be verified");
        return Err(CanonicalError::internal("failed to verify caller permissions").create());
    }

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
    fn a_page_about_a_person_is_stored_without_naming_them() {
        assert_eq!(
            normalize_path("/ic/cccccccc-0000-0000-0000-000000000001/personal/git_output"),
            "/ic/:id/personal/git_output"
        );
        assert_eq!(
            normalize_path("/portal/manage/platform-usage"),
            "/portal/manage/platform-usage"
        );
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
        // Both are already counted as visits; neither is something a person did.
        assert!(actions_sql("1").contains("NOT IN ('page_view', 'session_start')"));
    }

    #[test]
    fn a_visitor_is_named_from_the_mirrored_identity_rows() {
        let sql = people_sql();
        assert!(sql.contains("identity.identity_persons"), "{sql}");
        assert!(sql.contains("display_name"), "{sql}");
    }

    #[test]
    fn a_malformed_day_is_refused_rather_than_queried() {
        let query = UsageRangeQuery {
            since: Some("2026-99-99".to_owned()),
            until: None,
        };
        assert!(query.window().is_err(), "a date that cannot exist is a 400");

        let ok = UsageRangeQuery {
            since: Some("2026-01-31".to_owned()),
            until: Some("2026-02-01".to_owned()),
        };
        assert_eq!(
            ok.window().ok().map(|w| w.since.to_string()),
            Some("2026-01-31".to_owned())
        );
    }
}
