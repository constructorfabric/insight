use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::super::{AppState, clip};
use super::{PAGE_VIEW, TABLE};

const MAX_RECORDS: usize = 200;

/// The two `LowCardinality` columns, where an unbounded value blows the dictionary.
const MAX_NAME: usize = 64;

const MAX_FIELD: usize = 128;

const MAX_PATH: usize = 512;

/// How long a record may plausibly have waited in the browser's buffer. Past
/// this the correction cannot place it in the right day, which is the only
/// thing it exists to protect.
const MAX_BUFFERED_MS: i64 = 24 * 60 * 60 * 1000;

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

/// The SDK emits its own page view naming the location `url`; the app emits one
/// with `path`. Keeping both counts every page twice.
fn is_recordable(row: &UsageEventRow) -> bool {
    row.event_name != PAGE_VIEW || !row.path.is_empty()
}

#[cfg(test)]
mod tests;
