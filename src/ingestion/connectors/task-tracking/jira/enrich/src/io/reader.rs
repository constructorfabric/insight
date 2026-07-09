//! Read-side queries: Bronze events + snapshot; Silver high-water-marks + last state;
//! field metadata.

use super::ch_client::ChConfig;
use super::IoError;
use crate::core::types::{
    Delta, DeltaEvent, FieldCardinality, FieldMeta, FieldValue, IssueSnapshot, LastState,
    ValueIdType,
};
use chrono::{DateTime, TimeZone, Utc};
use clickhouse::query::RowCursor;
use clickhouse::Row;
use serde::Deserialize;
use std::collections::HashMap;

// ---------------- field metadata ----------------

#[derive(Row, Deserialize, Debug)]
struct FieldMetaRow {
    field_id: String,
    field_name: String,
    is_multi: u8,
    has_id: u8,
}

pub async fn fetch_field_metadata(
    cfg: &ChConfig,
    insight_source_id: &str,
) -> Result<HashMap<String, FieldMeta>, IoError> {
    let client = cfg.client();
    let rows: Vec<FieldMetaRow> = client
        .query(
            "SELECT field_id, field_name, is_multi, has_id \
             FROM staging.jira__task_field_metadata \
             WHERE data_source = 'jira' AND insight_source_id = ?",
        )
        .bind(insight_source_id)
        .fetch_all()
        .await?;

    let mut out = HashMap::with_capacity(rows.len());
    for r in rows {
        let cardinality = if r.is_multi == 1 {
            FieldCardinality::Multi
        } else {
            FieldCardinality::Single
        };
        let value_id_type = classify_value_id_type(&r.field_id, r.has_id);
        out.insert(
            r.field_id.clone(),
            FieldMeta {
                field_id: r.field_id,
                field_name: r.field_name,
                cardinality,
                value_id_type,
            },
        );
    }
    Ok(out)
}

fn classify_value_id_type(field_id: &str, has_id: u8) -> ValueIdType {
    match field_id {
        "labels" | "System.Tags" => ValueIdType::StringLiteral,
        "assignee" | "reporter" => ValueIdType::AccountId,
        _ if has_id == 1 => ValueIdType::OpaqueId,
        _ => ValueIdType::None,
    }
}

// ---------------- per-issue HWM ----------------

#[derive(Row, Deserialize, Debug)]
struct HwmRow {
    id_readable: String,
    hwm_ms: i64,
}

pub async fn per_issue_hwm(
    cfg: &ChConfig,
    insight_source_id: &str,
) -> Result<HashMap<String, DateTime<Utc>>, IoError> {
    let client = cfg.client();
    let rows: Vec<HwmRow> = client
        .query(
            "SELECT id_readable, toInt64(toUnixTimestamp64Milli(max(event_at))) AS hwm_ms \
             FROM staging.jira__task_field_history \
             WHERE data_source = 'jira' AND insight_source_id = ? \
             GROUP BY id_readable",
        )
        .bind(insight_source_id)
        .fetch_all()
        .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Utc.timestamp_millis_opt(r.hwm_ms)
                .single()
                .map(|t| (r.id_readable, t))
        })
        .collect())
}

// ---------------- last state per (id_readable, field_id) ----------------

#[derive(Row, Deserialize, Debug)]
struct LastStateRow {
    id_readable: String,
    field_id: String,
    value_ids: Vec<String>,
    value_displays: Vec<String>,
    last_event_at_ms: i64,
}

/// Chunk size for `id_readable IN (...)` lookups — keeps the serialized query under CH's
/// default `max_query_size` (256 KiB) even with long id_readable strings.
const ID_CHUNK: usize = 2_000;

pub async fn fetch_last_state_for(
    cfg: &ChConfig,
    insight_source_id: &str,
    issue_ids: &[String],
) -> Result<HashMap<String, HashMap<String, LastState>>, IoError> {
    if issue_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let client = cfg.client();
    let mut out: HashMap<String, HashMap<String, LastState>> = HashMap::new();

    for chunk in issue_ids.chunks(ID_CHUNK) {
        let chunk_vec = chunk.to_vec();
        let rows: Vec<LastStateRow> = client
            .clone()
            .query(
                "SELECT id_readable, field_id, \
                        argMax(value_ids, event_at)      AS value_ids, \
                        argMax(value_displays, event_at) AS value_displays, \
                        toInt64(toUnixTimestamp64Milli(max(event_at))) AS last_event_at_ms \
                 FROM staging.jira__task_field_history \
                 WHERE data_source = 'jira' \
                   AND insight_source_id = ? \
                   AND id_readable IN ? \
                 GROUP BY id_readable, field_id",
            )
            .bind(insight_source_id)
            .bind(chunk_vec)
            .fetch_all()
            .await?;

        for r in rows {
            let Some(last_at) = Utc.timestamp_millis_opt(r.last_event_at_ms).single() else {
                continue;
            };
            out.entry(r.id_readable).or_default().insert(
                r.field_id,
                LastState {
                    value: FieldValue {
                        ids: r.value_ids,
                        displays: r.value_displays,
                    },
                    last_event_at: last_at,
                },
            );
        }
    }
    Ok(out)
}

// ---------------- snapshots (bulk, in-memory) ----------------

#[derive(Row, Deserialize, Debug)]
struct IssueHeaderRow {
    insight_source_id: String,
    jira_id: String,
    id_readable: String,
    created_ms: i64,
    reporter_id: Option<String>,
}

#[derive(Row, Deserialize, Debug)]
struct IssueFieldRow {
    id_readable: String,
    field_id: String,
    value_ids: Vec<String>,
    value_displays: Vec<String>,
}

/// Bulk-load ALL issue snapshots with their per-field values from staging.
/// Used to populate `IssueSnapshot.current_fields` so bootstrap can emit synthetic_initial
/// rows for every field on every issue — including fields that never changed.
pub async fn fetch_all_snapshots(
    cfg: &ChConfig,
    insight_source_id: &str,
) -> Result<HashMap<String, IssueSnapshot>, IoError> {
    let client = cfg.client();

    // (a) Headers: one row per issue with core identity + created timestamp.
    let headers: Vec<IssueHeaderRow> = client
        .clone()
        .query(
            // FINAL forces ReplacingMergeTree merges on read. Bronze `jira_issue` is
            // append-only (Airbyte destinationSyncMode='append'); without FINAL the reader
            // can see multiple unmerged rows per issue when syncs overlap with merges.
            "SELECT COALESCE(source_id, '')                 AS insight_source_id, \
                    COALESCE(toString(jira_id), '')         AS jira_id, \
                    COALESCE(toString(id_readable), '')     AS id_readable, \
                    COALESCE(toInt64(toUnixTimestamp64Milli(parseDateTime64BestEffortOrNull(created, 3))), 0) AS created_ms, \
                    reporter_id \
             FROM bronze_jira.jira_issue AS ji FINAL \
             WHERE source_id = ?",
        )
        .bind(insight_source_id)
        .fetch_all()
        .await?;

    let mut out: HashMap<String, IssueSnapshot> = HashMap::with_capacity(headers.len());
    for h in headers {
        let Some(created) = Utc.timestamp_millis_opt(h.created_ms).single() else {
            continue;
        };
        out.insert(
            h.id_readable.clone(),
            IssueSnapshot {
                insight_source_id: h.insight_source_id,
                issue_id: h.jira_id,
                id_readable: h.id_readable,
                created_at: created,
                reporter_id: h.reporter_id,
                current_fields: HashMap::new(),
            },
        );
    }

    // (b) Per-field snapshot values from the dbt staging model.
    let fields: Vec<IssueFieldRow> = client
        .query(
            "SELECT id_readable, field_id, value_ids, value_displays \
             FROM staging.jira_issue_field_snapshot \
             WHERE insight_source_id = ?",
        )
        .bind(insight_source_id)
        .fetch_all()
        .await?;

    let mut attached = 0_usize;
    let mut orphan = 0_usize;
    for f in fields {
        if let Some(snap) = out.get_mut(&f.id_readable) {
            snap.current_fields.insert(
                f.field_id,
                crate::core::types::FieldValue {
                    ids: f.value_ids,
                    displays: f.value_displays,
                },
            );
            attached += 1;
        } else {
            orphan += 1;
        }
    }
    tracing::info!(
        snapshots_with_fields = attached,
        orphan_rows = orphan,
        "attached field snapshot values to issues"
    );

    Ok(out)
}

// ---------------- new events — streaming cursor ----------------

#[derive(Row, Deserialize, Debug)]
pub struct ChangelogRow {
    pub insight_source_id: String,
    pub issue_jira_id: String,
    pub id_readable: String,
    pub changelog_id: String,
    pub event_at_ms: i64,
    pub author_account_id: Option<String>,
    pub field_id: String,
    pub field_name: String,
    pub value_from: Option<String>,
    pub value_from_string: Option<String>,
    pub value_to: Option<String>,
    pub value_to_string: Option<String>,
}

/// Open a streaming cursor over new events — ordered by `(id_readable, event_at)` so callers
/// can group into per-issue chunks without buffering everything in memory.
pub fn open_events_cursor(
    cfg: &ChConfig,
    insight_source_id: &str,
) -> Result<RowCursor<ChangelogRow>, IoError> {
    let client = cfg.client();
    let cursor = client
        .query(
            "SELECT e.insight_source_id     AS insight_source_id, \
                    COALESCE(toString(i.jira_id), '') AS issue_jira_id, \
                    e.id_readable           AS id_readable, \
                    e.changelog_id          AS changelog_id, \
                    toInt64(toUnixTimestamp64Milli(e.created_at)) AS event_at_ms, \
                    e.author_account_id     AS author_account_id, \
                    e.field_id              AS field_id, \
                    e.field_name            AS field_name, \
                    e.value_from            AS value_from, \
                    e.value_from_string     AS value_from_string, \
                    e.value_to              AS value_to, \
                    e.value_to_string       AS value_to_string \
             FROM staging.jira_changelog_items e \
             LEFT JOIN bronze_jira.jira_issue AS i FINAL \
                 ON e.insight_source_id = i.source_id \
                AND e.id_readable        = i.id_readable \
             LEFT JOIN ( \
                 SELECT id_readable, max(event_at) AS hwm \
                 FROM staging.jira__task_field_history \
                 WHERE data_source = 'jira' AND insight_source_id = ? \
                 GROUP BY id_readable \
             ) c ON e.id_readable = c.id_readable \
             WHERE e.insight_source_id = ? \
               AND (c.hwm IS NULL OR e.created_at > c.hwm) \
             ORDER BY e.id_readable, e.created_at, e.changelog_id, e.field_id",
        )
        .bind(insight_source_id)
        .bind(insight_source_id)
        .fetch::<ChangelogRow>()?;
    Ok(cursor)
}

/// Convert a wire-row into a domain `DeltaEvent`, applying Jira-specific delta semantics.
#[must_use]
pub fn row_to_event(r: ChangelogRow, meta: &HashMap<String, FieldMeta>) -> Option<DeltaEvent> {
    let event_at = Utc.timestamp_millis_opt(r.event_at_ms).single()?;
    let cardinality = meta
        .get(&r.field_id)
        .map(|m| m.cardinality)
        .unwrap_or(FieldCardinality::Single);
    let item = crate::core::jira::BronzeChangelogItem {
        field_id: &r.field_id,
        value_from: r.value_from.as_deref(),
        value_from_string: r.value_from_string.as_deref(),
        value_to: r.value_to.as_deref(),
        value_to_string: r.value_to_string.as_deref(),
    };
    let delta = crate::core::jira::to_delta(&item, cardinality)?;
    Some(DeltaEvent {
        insight_source_id: r.insight_source_id,
        issue_id: r.issue_jira_id,
        id_readable: r.id_readable,
        event_id: r.changelog_id,
        event_at,
        author_id: r.author_account_id,
        field_id: r.field_id,
        field_name: r.field_name,
        delta,
    })
}

// Force deserialization path referenced.
#[allow(dead_code)]
fn _mark_used(_d: &Delta) {}
