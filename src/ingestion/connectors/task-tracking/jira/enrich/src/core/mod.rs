pub mod jira;
pub mod types;

use std::collections::{BTreeMap, HashMap};

use types::{
    synthetic_initial_event_id, DataSource, Delta, DeltaAction, DeltaEvent, EventKind,
    FieldCardinality, FieldHistoryRecord, FieldId, FieldMeta, FieldValue, IssueSnapshot, LastState,
};

/// Sentinel `field_id` for the per-issue creation marker. NOT a real Jira field — downstream
/// field-filters (which only match real fields like status/assignee/…) ignore it, and the only
/// consumer that picks it up is `task_issue_current_state.created_at = minIf(event_at,
/// event_kind = 'synthetic_initial')`, where it carries the issue's creation timestamp.
const CREATED_FIELD_ID: &str = "created";

pub fn process_issue(
    meta: &HashMap<FieldId, FieldMeta>,
    snapshot: &IssueSnapshot,
    events_sorted: &[DeltaEvent],
    existing: Option<&HashMap<FieldId, LastState>>,
) -> Vec<FieldHistoryRecord> {
    match existing {
        None => bootstrap(meta, snapshot, events_sorted),
        Some(existing_state) => incremental(meta, events_sorted, existing_state),
    }
}

/// Bootstrap path — called when the issue has no rows yet in `task_tracker_field_history`.
///
/// Emits, in order:
///   1. ONE **creation marker** row (`field_id = "created"`, `event_kind = synthetic_initial`,
///      `seq = 0`) representing "the task was created". This is a cross-source contract: it is
///      the single, distinct, stably-ordered record of issue creation. (When the `YouTrack`
///      enrich is built it MUST emit the same marker so silver `class_task_field_history` is
///      uniform across sources.) The marker is a pure who/when row — `author_id`/`event_at`
///      carry the reporter and creation time; it has no value. `"created"` is a sentinel, not a
///      real Jira field, so downstream field-filters ignore it; only
///      `task_issue_current_state.created_at` (a `minIf` over `synthetic_initial`) reads it.
///   2. A `synthetic_initial` row for **every** field present in the snapshot, including ones
///      that were never touched by the changelog, with `seq = 1 ..= N` in `field_id`-ASC order.
///      Fields that did change are reverse-applied so the initial row shows the *original* value
///      (before the earliest changelog event), not the current value.
///   3. The changelog rows (`event_kind = changelog`, `seq = 0`; they sort after by `event_at`).
fn bootstrap(
    meta: &HashMap<FieldId, FieldMeta>,
    snapshot: &IssueSnapshot,
    events_sorted: &[DeltaEvent],
) -> Vec<FieldHistoryRecord> {
    // Start from snapshot (current state for all fields), then reverse the changelog in
    // order to roll it back to the state at issue creation.
    let initial = reconstruct_initial(meta, snapshot, events_sorted);

    // Deterministic order: fields sorted by field_id ASC, `seq` is the index.
    let mut ordered: BTreeMap<&FieldId, &FieldValue> = initial.iter().collect();

    let mut out = Vec::with_capacity(1 + ordered.len() + events_sorted.len());

    // Per-issue creation marker is always the first row (seq = 0).
    out.push(emit_creation_row(snapshot));

    // Per-field synthetic_initial rows follow, starting at seq = 1.
    for (seq, (field_id, value)) in ordered.iter_mut().enumerate() {
        let seq = seq + 1;
        let meta_entry = meta.get(*field_id);
        let (cardinality, value_id_type, field_name) = if let Some(m) = meta_entry {
            (m.cardinality, m.value_id_type, m.field_name.clone())
        } else {
            // Field not in metadata — infer cardinality from shape, use None for id_type.
            let card = if value.ids.len() > 1 {
                FieldCardinality::Multi
            } else {
                FieldCardinality::Single
            };
            (card, types::ValueIdType::None, (*field_id).clone())
        };
        out.push(emit_synthetic_initial_row(
            snapshot,
            field_id,
            &field_name,
            cardinality,
            value_id_type,
            value,
            u32::try_from(seq).unwrap_or(u32::MAX),
        ));
    }

    // Forward-apply events to produce changelog rows with running state.
    let mut state: HashMap<FieldId, FieldValue> = initial;
    for ev in events_sorted {
        let Some(field_meta) = meta.get(&ev.field_id) else {
            tracing::warn!(field_id = %ev.field_id, "event references unknown field — skipping");
            continue;
        };
        let prev = state.remove(&ev.field_id).unwrap_or_else(FieldValue::empty);
        let next = apply_delta(prev, &ev.delta, field_meta.cardinality);
        out.push(emit_changelog_row(ev, field_meta, &next));
        state.insert(ev.field_id.clone(), next);
    }

    out
}

/// Incremental path — per-issue HWM exists in `existing`. No synthetic rows emitted (they
/// are already in silver from a prior bootstrap). Only forward-apply new events.
fn incremental(
    meta: &HashMap<FieldId, FieldMeta>,
    events_sorted: &[DeltaEvent],
    existing: &HashMap<FieldId, LastState>,
) -> Vec<FieldHistoryRecord> {
    let hwm = existing
        .values()
        .map(|s| s.last_event_at)
        .max()
        .unwrap_or_default();

    let cutoff = events_sorted.partition_point(|ev| ev.event_at <= hwm);
    let new_events = &events_sorted[cutoff..];

    let mut state: HashMap<FieldId, FieldValue> = existing
        .iter()
        .map(|(k, v)| (k.clone(), v.value.clone()))
        .collect();

    let mut out = Vec::with_capacity(new_events.len());
    for ev in new_events {
        let Some(field_meta) = meta.get(&ev.field_id) else {
            tracing::warn!(field_id = %ev.field_id, "event references unknown field — skipping");
            continue;
        };
        let prev = state.remove(&ev.field_id).unwrap_or_else(FieldValue::empty);
        let next = apply_delta(prev, &ev.delta, field_meta.cardinality);
        out.push(emit_changelog_row(ev, field_meta, &next));
        state.insert(ev.field_id.clone(), next);
    }

    out
}

fn reconstruct_initial(
    meta: &HashMap<FieldId, FieldMeta>,
    snapshot: &IssueSnapshot,
    events_sorted: &[DeltaEvent],
) -> HashMap<FieldId, FieldValue> {
    let mut state: HashMap<FieldId, FieldValue> = snapshot.current_fields.clone();

    for ev in events_sorted.iter().rev() {
        let Some(field_meta) = meta.get(&ev.field_id) else {
            continue;
        };
        let prev = state.remove(&ev.field_id).unwrap_or_else(FieldValue::empty);
        let before = reverse_delta(prev, &ev.delta, field_meta.cardinality);
        state.insert(ev.field_id.clone(), before);
    }

    state
}

pub(crate) fn apply_delta(
    state: FieldValue,
    delta: &Delta,
    cardinality: FieldCardinality,
) -> FieldValue {
    match (delta, cardinality) {
        (Delta::Set { to, to_display, .. }, _) => match (to, to_display) {
            (Some(id), Some(disp)) => FieldValue {
                ids: vec![id.clone()],
                displays: vec![disp.clone()],
            },
            (Some(id), None) => FieldValue {
                ids: vec![id.clone()],
                displays: vec![id.clone()],
            },
            _ => FieldValue::empty(),
        },
        (
            Delta::Snapshot {
                to_ids,
                to_displays,
                ..
            },
            _,
        ) => FieldValue {
            ids: to_ids.clone(),
            displays: to_displays.clone(),
        },
        (Delta::Add { id, display }, FieldCardinality::Multi) => {
            let mut s = state;
            if !s.ids.contains(id) {
                s.ids.push(id.clone());
                s.displays.push(display.clone());
            }
            s
        }
        (Delta::Remove { id, display: _ }, FieldCardinality::Multi) => {
            let mut s = state;
            if let Some(pos) = s.ids.iter().position(|i| i == id) {
                s.ids.remove(pos);
                s.displays.remove(pos);
            }
            s
        }
        (Delta::Add { id, display }, FieldCardinality::Single) => FieldValue {
            ids: vec![id.clone()],
            displays: vec![display.clone()],
        },
        (Delta::Remove { .. }, FieldCardinality::Single) => FieldValue::empty(),
    }
}

pub(crate) fn reverse_delta(
    state: FieldValue,
    delta: &Delta,
    cardinality: FieldCardinality,
) -> FieldValue {
    match (delta, cardinality) {
        (
            Delta::Set {
                from, from_display, ..
            },
            _,
        ) => match (from, from_display) {
            (Some(id), Some(disp)) => FieldValue {
                ids: vec![id.clone()],
                displays: vec![disp.clone()],
            },
            (Some(id), None) => FieldValue {
                ids: vec![id.clone()],
                displays: vec![id.clone()],
            },
            _ => FieldValue::empty(),
        },
        (
            Delta::Snapshot {
                from_ids,
                from_displays,
                ..
            },
            _,
        ) => FieldValue {
            ids: from_ids.clone(),
            displays: from_displays.clone(),
        },
        (Delta::Add { id, display: _ }, FieldCardinality::Multi) => {
            let mut s = state;
            if let Some(pos) = s.ids.iter().position(|i| i == id) {
                s.ids.remove(pos);
                s.displays.remove(pos);
            }
            s
        }
        (Delta::Remove { id, display }, FieldCardinality::Multi) => {
            let mut s = state;
            if !s.ids.contains(id) {
                s.ids.push(id.clone());
                s.displays.push(display.clone());
            }
            s
        }
        (Delta::Add { .. } | Delta::Remove { .. }, FieldCardinality::Single) => FieldValue::empty(),
    }
}

/// Per-issue creation marker. A synthetic `event_kind = synthetic_initial` row with the
/// sentinel `field_id = "created"`, `seq = 0`, no value — it records only *who* created the
/// issue (`author_id = reporter`) and *when* (`event_at = created_at`). Shares the
/// `initial:<issue_id>` `event_id` with the per-field synthetic rows; `unique_key` stays distinct
/// because `field_id` differs. See `bootstrap` for the cross-source contract.
fn emit_creation_row(snapshot: &IssueSnapshot) -> FieldHistoryRecord {
    FieldHistoryRecord {
        insight_source_id: snapshot.insight_source_id.clone(),
        data_source: DataSource::Jira,
        issue_id: snapshot.issue_id.clone(),
        id_readable: snapshot.id_readable.clone(),
        event_id: synthetic_initial_event_id(&snapshot.issue_id),
        event_at: snapshot.created_at,
        event_kind: EventKind::SyntheticInitial,
        seq: 0,
        author_id: snapshot.reporter_id.clone(),
        author_display: None,
        field_id: CREATED_FIELD_ID.to_owned(),
        field_name: "Created".to_owned(),
        field_cardinality: FieldCardinality::Single,
        delta_action: DeltaAction::Set,
        delta_value_id: None,
        delta_value_display: None,
        value_ids: Vec::new(),
        value_displays: Vec::new(),
        value_id_type: types::ValueIdType::None,
    }
}

fn emit_synthetic_initial_row(
    snapshot: &IssueSnapshot,
    field_id: &str,
    field_name: &str,
    cardinality: FieldCardinality,
    value_id_type: types::ValueIdType,
    value: &FieldValue,
    seq: u32,
) -> FieldHistoryRecord {
    FieldHistoryRecord {
        insight_source_id: snapshot.insight_source_id.clone(),
        data_source: DataSource::Jira,
        issue_id: snapshot.issue_id.clone(),
        id_readable: snapshot.id_readable.clone(),
        event_id: synthetic_initial_event_id(&snapshot.issue_id),
        event_at: snapshot.created_at,
        event_kind: EventKind::SyntheticInitial,
        seq,
        author_id: snapshot.reporter_id.clone(),
        author_display: None,
        field_id: field_id.to_owned(),
        field_name: field_name.to_owned(),
        field_cardinality: cardinality,
        delta_action: match cardinality {
            FieldCardinality::Single => DeltaAction::Set,
            FieldCardinality::Multi => DeltaAction::Add,
        },
        delta_value_id: value.ids.first().cloned(),
        delta_value_display: value.displays.first().cloned(),
        value_ids: value.ids.clone(),
        value_displays: value.displays.clone(),
        value_id_type,
    }
}

fn emit_changelog_row(
    ev: &DeltaEvent,
    meta: &FieldMeta,
    state_after: &FieldValue,
) -> FieldHistoryRecord {
    let (delta_action, delta_value_id, delta_value_display) = match &ev.delta {
        Delta::Set { to, to_display, .. } => (DeltaAction::Set, to.clone(), to_display.clone()),
        Delta::Add { id, display } => (DeltaAction::Add, Some(id.clone()), Some(display.clone())),
        Delta::Remove { id, display } => {
            (DeltaAction::Remove, Some(id.clone()), Some(display.clone()))
        }
        Delta::Snapshot {
            to_ids,
            to_displays,
            ..
        } => (
            DeltaAction::Set,
            to_ids.first().cloned(),
            to_displays.first().cloned(),
        ),
    };

    FieldHistoryRecord {
        insight_source_id: ev.insight_source_id.clone(),
        data_source: DataSource::Jira,
        issue_id: ev.issue_id.clone(),
        id_readable: ev.id_readable.clone(),
        event_id: ev.event_id.clone(),
        event_at: ev.event_at,
        event_kind: EventKind::Changelog,
        seq: 0,
        author_id: ev.author_id.clone(),
        author_display: None,
        field_id: meta.field_id.clone(),
        field_name: meta.field_name.clone(),
        field_cardinality: meta.cardinality,
        delta_action,
        delta_value_id,
        delta_value_display,
        value_ids: state_after.ids.clone(),
        value_displays: state_after.displays.clone(),
        value_id_type: meta.value_id_type,
    }
}

#[cfg(test)]
mod tests;
