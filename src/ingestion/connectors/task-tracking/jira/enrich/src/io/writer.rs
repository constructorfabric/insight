//! Batched INSERT into `staging.jira__task_field_history` (which dbt then unions
//! into `silver.class_task_field_history` via `union_by_tag`).
//!
//! One binary run fills `_version = collected_at_ms` so ReplacingMergeTree resolves
//! duplicates from overlapping runs deterministically (last-write-wins by run time).

use super::IoError;
use super::ch_client::ChConfig;
use crate::core::types::{
    DataSource, DeltaAction, EventKind, FieldCardinality, FieldHistoryRecord, ValueIdType,
};
use chrono::{DateTime, Utc};
use clickhouse::Row;
use serde::Serialize;

/// Wire-format row. Matches `staging.jira__task_field_history` DDL from the macro.
/// Enum8 columns are serialized as i8 (the ClickHouse wire format expects the numeric
/// enum value, not the string label).
#[derive(Row, Serialize, Debug)]
pub struct FieldHistoryInsert {
    /// Project-wide convention key for ReplacingMergeTree dedup. Synthesized from
    /// (insight_source_id, data_source, id_readable, field_id, event_id) — these
    /// five components together uniquely identify one (issue × field × event)
    /// per ADR-005. Same formula as connector AddFields would produce if the
    /// staging table were Airbyte-managed.
    pub unique_key: String,
    pub insight_source_id: String,
    pub data_source: String,
    pub issue_id: String,
    pub id_readable: String,
    pub event_id: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    pub event_at: DateTime<Utc>,
    pub event_kind: i8,         // Enum8('changelog'=1, 'synthetic_initial'=2)
    #[serde(rename = "_seq")]
    pub seq: u32,
    pub author_id: Option<String>,
    pub author_display: Option<String>,
    pub field_id: String,
    pub field_name: String,
    pub field_cardinality: i8,  // Enum8('single'=1, 'multi'=2)
    pub delta_action: i8,       // Enum8('set'=1, 'add'=2, 'remove'=3)
    pub delta_value_id: Option<String>,
    pub delta_value_display: Option<String>,
    pub value_ids: Vec<String>,
    pub value_displays: Vec<String>,
    pub value_id_type: i8,      // Enum8('opaque_id'=1, 'account_id'=2, 'string_literal'=3, 'path'=4, 'none'=5)
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    pub collected_at: DateTime<Utc>,
    pub _version: u64,
}

// @cpt-principle:cpt-dataflow-principle-unique-key-formula:p1
impl From<FieldHistoryRecord> for FieldHistoryInsert {
    fn from(r: FieldHistoryRecord) -> Self {
        let collected_at = Utc::now();
        let version = u64::try_from(collected_at.timestamp_millis().max(0)).unwrap_or(0);
        let data_source = data_source_str(r.data_source);
        let unique_key = format!(
            "{}-{}-{}-{}-{}",
            r.insight_source_id, data_source, r.id_readable, r.field_id, r.event_id
        );
        Self {
            unique_key,
            insight_source_id: r.insight_source_id,
            data_source: data_source.into(),
            issue_id: r.issue_id,
            id_readable: r.id_readable,
            event_id: r.event_id,
            event_at: r.event_at,
            event_kind: event_kind_enum(r.event_kind),
            seq: r.seq,
            author_id: r.author_id,
            author_display: r.author_display,
            field_id: r.field_id,
            field_name: r.field_name,
            field_cardinality: cardinality_enum(r.field_cardinality),
            delta_action: delta_action_enum(r.delta_action),
            delta_value_id: r.delta_value_id,
            delta_value_display: r.delta_value_display,
            value_ids: r.value_ids,
            value_displays: r.value_displays,
            value_id_type: value_id_type_enum(r.value_id_type),
            collected_at,
            _version: version,
        }
    }
}

fn data_source_str(d: DataSource) -> &'static str {
    match d {
        DataSource::Jira => "jira",
    }
}

fn event_kind_enum(k: EventKind) -> i8 {
    match k {
        EventKind::Changelog => 1,
        EventKind::SyntheticInitial => 2,
    }
}

fn cardinality_enum(c: FieldCardinality) -> i8 {
    match c {
        FieldCardinality::Single => 1,
        FieldCardinality::Multi => 2,
    }
}

fn delta_action_enum(a: DeltaAction) -> i8 {
    match a {
        DeltaAction::Set => 1,
        DeltaAction::Add => 2,
        DeltaAction::Remove => 3,
    }
}

fn value_id_type_enum(t: ValueIdType) -> i8 {
    match t {
        ValueIdType::OpaqueId => 1,
        ValueIdType::AccountId => 2,
        ValueIdType::StringLiteral => 3,
        ValueIdType::Path => 4,
        ValueIdType::None => 5,
    }
}

pub async fn insert_batch(
    cfg: &ChConfig,
    rows: Vec<FieldHistoryRecord>,
) -> Result<usize, IoError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let client = cfg.client();
    let mut inserter = client
        .insert::<FieldHistoryInsert>("jira__task_field_history")
        .await?;
    let len = rows.len();
    for r in rows {
        inserter.write(&FieldHistoryInsert::from(r)).await?;
    }
    inserter.end().await?;
    Ok(len)
}
