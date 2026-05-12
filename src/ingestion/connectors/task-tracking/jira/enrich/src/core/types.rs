use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type FieldId = String;
pub type IssueId = String;
pub type IdReadable = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueIdType {
    OpaqueId,
    AccountId,
    StringLiteral,
    Path,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldCardinality {
    Single,
    Multi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaAction {
    Set,
    Add,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Changelog,
    /// Synthesized from `bronze_jira.jira_issue` snapshot (not from real changelog).
    /// Emitted once per (issue, field) that the issue has at creation time — including
    /// fields that never changed. `event_at = issue.created`, ordering disambiguated by `seq`.
    SyntheticInitial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    Jira,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMeta {
    pub field_id: FieldId,
    pub field_name: String,
    pub cardinality: FieldCardinality,
    pub value_id_type: ValueIdType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldValue {
    pub ids: Vec<String>,
    pub displays: Vec<String>,
}

impl FieldValue {
    #[must_use]
    pub fn empty() -> Self {
        Self { ids: Vec::new(), displays: Vec::new() }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueSnapshot {
    pub insight_source_id: String,
    pub issue_id: IssueId,
    pub id_readable: IdReadable,
    pub created_at: DateTime<Utc>,
    pub reporter_id: Option<String>,
    pub current_fields: HashMap<FieldId, FieldValue>,
}

/// A single field change. Carries both sides (from / to) so the same value can be applied
/// forward (during forward_apply) or reverse (during reconstruct::reverse_apply).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Delta {
    /// Single-value replacement. Either side may be NULL (field was empty / became empty).
    Set {
        from: Option<String>,
        from_display: Option<String>,
        to: Option<String>,
        to_display: Option<String>,
    },
    /// Multi-value: add an item. Reverse = remove the same item.
    Add { id: String, display: String },
    /// Multi-value: remove an item. Reverse = add it back.
    Remove { id: String, display: String },
    /// Full-snapshot replacement (Jira Sprint: `toString` = full list, `fromString` = old list).
    Snapshot {
        from_ids: Vec<String>,
        from_displays: Vec<String>,
        to_ids: Vec<String>,
        to_displays: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaEvent {
    pub insight_source_id: String,
    pub issue_id: IssueId,
    pub id_readable: IdReadable,
    pub event_id: String,
    pub event_at: DateTime<Utc>,
    pub author_id: Option<String>,
    pub field_id: FieldId,
    pub field_name: String,
    pub delta: Delta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastState {
    pub value: FieldValue,
    pub last_event_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldHistoryRecord {
    pub insight_source_id: String,
    pub data_source: DataSource,
    pub issue_id: IssueId,
    pub id_readable: IdReadable,
    pub event_id: String,
    pub event_at: DateTime<Utc>,
    pub event_kind: EventKind,
    /// Secondary sort key. 0 for changelog rows. For synthetic_initial rows: the 0-based
    /// index of the field in the sorted (by `field_id` ASC) list of issue fields — so
    /// consumers sorting by `(event_at, _seq)` get stable deterministic order.
    pub seq: u32,
    pub author_id: Option<String>,
    pub author_display: Option<String>,
    pub field_id: FieldId,
    pub field_name: String,
    pub field_cardinality: FieldCardinality,
    pub delta_action: DeltaAction,
    pub delta_value_id: Option<String>,
    pub delta_value_display: Option<String>,
    pub value_ids: Vec<String>,
    pub value_displays: Vec<String>,
    pub value_id_type: ValueIdType,
}

#[must_use]
pub fn synthetic_initial_event_id(issue_id: &str) -> String {
    format!("initial:{issue_id}")
}
