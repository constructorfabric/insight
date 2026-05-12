pub mod jira;
pub mod types;

use std::collections::{BTreeMap, HashMap};

use types::{
    DataSource, Delta, DeltaAction, DeltaEvent, EventKind, FieldCardinality, FieldHistoryRecord,
    FieldId, FieldMeta, FieldValue, IssueSnapshot, LastState, synthetic_initial_event_id,
};

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
/// Emits a `synthetic_initial` row for **every** field present in the snapshot, including
/// ones that were never touched by the changelog. Fields that did change are additionally
/// reverse-applied so that the initial row shows the *original* value (before the earliest
/// changelog event), not the current value.
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

    let mut out = Vec::with_capacity(ordered.len() + events_sorted.len());
    for (seq, (field_id, value)) in ordered.iter_mut().enumerate() {
        let meta_entry = meta.get(*field_id);
        let (cardinality, value_id_type, field_name) = match meta_entry {
            Some(m) => (m.cardinality, m.value_id_type, m.field_name.clone()),
            None => {
                // Field not in metadata — infer cardinality from shape, use None for id_type.
                let card = if value.ids.len() > 1 {
                    FieldCardinality::Multi
                } else {
                    FieldCardinality::Single
                };
                (card, types::ValueIdType::None, (*field_id).clone())
            }
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

    let mut state: HashMap<FieldId, FieldValue> =
        existing.iter().map(|(k, v)| (k.clone(), v.value.clone())).collect();

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
        (Delta::Snapshot { to_ids, to_displays, .. }, _) => FieldValue {
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
        (Delta::Set { from, from_display, .. }, _) => match (from, from_display) {
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
        (Delta::Snapshot { from_ids, from_displays, .. }, _) => FieldValue {
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
        Delta::Set { to, to_display, .. } => {
            (DeltaAction::Set, to.clone(), to_display.clone())
        }
        Delta::Add { id, display } => {
            (DeltaAction::Add, Some(id.clone()), Some(display.clone()))
        }
        Delta::Remove { id, display } => {
            (DeltaAction::Remove, Some(id.clone()), Some(display.clone()))
        }
        Delta::Snapshot { to_ids, to_displays, .. } => (
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
