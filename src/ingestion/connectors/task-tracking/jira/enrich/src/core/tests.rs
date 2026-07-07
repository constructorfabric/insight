use super::types::{
    synthetic_initial_event_id, DataSource, Delta, DeltaAction, DeltaEvent, EventKind,
    FieldCardinality, FieldMeta, FieldValue, IssueSnapshot, LastState, ValueIdType,
};
use super::{apply_delta, process_issue, reverse_delta};
use chrono::{DateTime, TimeZone, Utc};
use std::collections::HashMap;

fn ts(y: i32, m: u32, d: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, hour, 0, 0).single().unwrap()
}

fn meta_status() -> FieldMeta {
    FieldMeta {
        field_id: "status".into(),
        field_name: "Status".into(),
        cardinality: FieldCardinality::Single,
        value_id_type: ValueIdType::OpaqueId,
    }
}

fn meta_labels() -> FieldMeta {
    FieldMeta {
        field_id: "labels".into(),
        field_name: "Labels".into(),
        cardinality: FieldCardinality::Multi,
        value_id_type: ValueIdType::StringLiteral,
    }
}

fn meta_sprint() -> FieldMeta {
    FieldMeta {
        field_id: "customfield_10020".into(),
        field_name: "Sprint".into(),
        cardinality: FieldCardinality::Multi,
        value_id_type: ValueIdType::OpaqueId,
    }
}

fn set(from: Option<&str>, to: Option<&str>) -> Delta {
    Delta::Set {
        from: from.map(str::to_owned),
        from_display: from.map(str::to_owned),
        to: to.map(str::to_owned),
        to_display: to.map(str::to_owned),
    }
}

fn set_full(from: Option<(&str, &str)>, to: Option<(&str, &str)>) -> Delta {
    Delta::Set {
        from: from.map(|(id, _)| id.to_owned()),
        from_display: from.map(|(_, d)| d.to_owned()),
        to: to.map(|(id, _)| id.to_owned()),
        to_display: to.map(|(_, d)| d.to_owned()),
    }
}

fn ev(event_id: &str, event_at: DateTime<Utc>, field: &FieldMeta, delta: Delta) -> DeltaEvent {
    DeltaEvent {
        insight_source_id: "jira-alpha".into(),
        issue_id: "10042".into(),
        id_readable: "PROJ-123".into(),
        event_id: event_id.into(),
        event_at,
        author_id: Some("acc-2".into()),
        field_id: field.field_id.clone(),
        field_name: field.field_name.clone(),
        delta,
    }
}

fn snap(current: HashMap<String, FieldValue>, created: DateTime<Utc>) -> IssueSnapshot {
    IssueSnapshot {
        insight_source_id: "jira-alpha".into(),
        issue_id: "10042".into(),
        id_readable: "PROJ-123".into(),
        created_at: created,
        reporter_id: Some("acc-1".into()),
        current_fields: current,
    }
}

fn meta_map() -> HashMap<String, FieldMeta> {
    [meta_status(), meta_labels(), meta_sprint()]
        .into_iter()
        .map(|m| (m.field_id.clone(), m))
        .collect()
}

// ---------------- low-level apply/reverse ----------------

#[test]
fn synthetic_event_id_is_deterministic() {
    assert_eq!(synthetic_initial_event_id("10042"), "initial:10042");
}

#[test]
fn apply_set_replaces_single_value() {
    let initial = FieldValue {
        ids: vec!["1".into()],
        displays: vec!["To Do".into()],
    };
    let out = apply_delta(
        initial,
        &set(Some("1"), Some("3")),
        FieldCardinality::Single,
    );
    assert_eq!(out.ids, vec!["3".to_string()]);
}

#[test]
fn apply_set_to_none_clears_value() {
    let initial = FieldValue {
        ids: vec!["x".into()],
        displays: vec!["x".into()],
    };
    let out = apply_delta(initial, &set(Some("x"), None), FieldCardinality::Single);
    assert!(out.is_empty());
}

#[test]
fn apply_add_appends_and_dedups() {
    let initial = FieldValue {
        ids: vec!["urgent".into()],
        displays: vec!["urgent".into()],
    };
    let first = apply_delta(
        initial,
        &Delta::Add {
            id: "backend".into(),
            display: "backend".into(),
        },
        FieldCardinality::Multi,
    );
    assert_eq!(first.ids, vec!["urgent".to_string(), "backend".to_string()]);

    let second = apply_delta(
        first,
        &Delta::Add {
            id: "urgent".into(),
            display: "urgent".into(),
        },
        FieldCardinality::Multi,
    );
    assert_eq!(
        second.ids,
        vec!["urgent".to_string(), "backend".to_string()]
    );
}

#[test]
fn apply_remove_drops_by_id() {
    let initial = FieldValue {
        ids: vec!["urgent".into(), "backend".into()],
        displays: vec!["urgent".into(), "backend".into()],
    };
    let out = apply_delta(
        initial,
        &Delta::Remove {
            id: "urgent".into(),
            display: "urgent".into(),
        },
        FieldCardinality::Multi,
    );
    assert_eq!(out.ids, vec!["backend".to_string()]);
}

#[test]
fn apply_snapshot_uses_to_side() {
    let out = apply_delta(
        FieldValue::empty(),
        &Delta::Snapshot {
            from_ids: vec!["old".into()],
            from_displays: vec!["Old".into()],
            to_ids: vec!["24".into(), "25".into()],
            to_displays: vec!["Sprint 24".into(), "Sprint 25".into()],
        },
        FieldCardinality::Multi,
    );
    assert_eq!(out.ids, vec!["24".to_string(), "25".to_string()]);
}

#[test]
fn reverse_set_returns_from_side() {
    let state_after = FieldValue {
        ids: vec!["3".into()],
        displays: vec!["Done".into()],
    };
    let before = reverse_delta(
        state_after,
        &set(Some("2"), Some("3")),
        FieldCardinality::Single,
    );
    assert_eq!(before.ids, vec!["2".to_string()]);
}

#[test]
fn reverse_add_removes_the_item() {
    let state_after = FieldValue {
        ids: vec!["urgent".into(), "backend".into()],
        displays: vec!["urgent".into(), "backend".into()],
    };
    let before = reverse_delta(
        state_after,
        &Delta::Add {
            id: "backend".into(),
            display: "backend".into(),
        },
        FieldCardinality::Multi,
    );
    assert_eq!(before.ids, vec!["urgent".to_string()]);
}

#[test]
fn reverse_remove_adds_the_item_back() {
    let state_after = FieldValue {
        ids: vec!["urgent".into()],
        displays: vec!["urgent".into()],
    };
    let before = reverse_delta(
        state_after,
        &Delta::Remove {
            id: "backend".into(),
            display: "backend".into(),
        },
        FieldCardinality::Multi,
    );
    assert_eq!(
        before.ids,
        vec!["urgent".to_string(), "backend".to_string()]
    );
}

// ---------------- process_issue bootstrap ----------------

#[test]
fn bootstrap_reconstructs_initial_state() {
    let meta = meta_map();
    let status = meta_status();
    let labels = meta_labels();

    // Final state: status=Done (id=3), labels=[backend, urgent]
    let snapshot = snap(
        HashMap::from([
            (
                "status".to_string(),
                FieldValue {
                    ids: vec!["3".into()],
                    displays: vec!["Done".into()],
                },
            ),
            (
                "labels".to_string(),
                FieldValue {
                    ids: vec!["backend".into(), "urgent".into()],
                    displays: vec!["backend".into(), "urgent".into()],
                },
            ),
        ]),
        ts(2026, 1, 1, 10),
    );

    // Events (chronological):
    //   1) status: To Do (1) → In Progress (2)
    //   2) labels: Add "backend"
    //   3) status: In Progress (2) → Done (3)
    //   4) labels: Add "urgent"
    let events = vec![
        ev(
            "cl-1",
            ts(2026, 1, 2, 9),
            &status,
            set_full(Some(("1", "To Do")), Some(("2", "In Progress"))),
        ),
        ev(
            "cl-2",
            ts(2026, 1, 3, 9),
            &labels,
            Delta::Add {
                id: "backend".into(),
                display: "backend".into(),
            },
        ),
        ev(
            "cl-3",
            ts(2026, 1, 4, 9),
            &status,
            set_full(Some(("2", "In Progress")), Some(("3", "Done"))),
        ),
        ev(
            "cl-4",
            ts(2026, 1, 5, 9),
            &labels,
            Delta::Add {
                id: "urgent".into(),
                display: "urgent".into(),
            },
        ),
    ];

    let out = process_issue(&meta, &snapshot, &events, None);

    // 1 creation marker + 2 synthetic_initial rows (labels=empty sorted first by field_id,
    // status=To Do) + 4 changelog.
    assert_eq!(out.len(), 7);

    // Creation marker is always first (seq=0).
    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[0].field_id, "created");
    assert_eq!(out[0].seq, 0);
    assert_eq!(out[0].event_id, "initial:10042");

    // synthetic_initial rows are sorted by field_id ASC → "labels" comes before "status",
    // now starting at seq=1.
    assert_eq!(out[1].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[1].field_id, "labels");
    assert_eq!(out[1].seq, 1);
    assert!(out[1].value_displays.is_empty()); // empty labels at creation
    assert_eq!(out[1].event_id, "initial:10042");

    assert_eq!(out[2].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[2].field_id, "status");
    assert_eq!(out[2].seq, 2);
    assert_eq!(out[2].value_displays, vec!["To Do".to_string()]);

    // Changelog events follow, in chronological order (seq=0 for all changelog).
    assert_eq!(out[3].event_id, "cl-1");
    assert_eq!(out[3].value_ids, vec!["2".to_string()]);
    assert_eq!(out[3].seq, 0);

    assert_eq!(out[4].event_id, "cl-2");
    assert_eq!(out[4].delta_action, DeltaAction::Add);
    assert_eq!(out[4].value_ids, vec!["backend".to_string()]);

    assert_eq!(out[5].event_id, "cl-3");
    assert_eq!(out[5].value_ids, vec!["3".to_string()]);

    assert_eq!(out[6].event_id, "cl-4");
    assert_eq!(
        out[6].value_ids,
        vec!["backend".to_string(), "urgent".to_string()]
    );
}

#[test]
fn bootstrap_emits_only_initial_when_changelog_empty() {
    let meta = meta_map();
    let snapshot = snap(
        HashMap::from([(
            "status".to_string(),
            FieldValue {
                ids: vec!["1".into()],
                displays: vec!["To Do".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    let out = process_issue(&meta, &snapshot, &[], None);
    // 1 creation marker (seq 0) + 1 synthetic_initial for status (seq 1).
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[0].field_id, "created");
    assert_eq!(out[0].seq, 0);
    assert_eq!(out[1].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[1].field_id, "status");
    assert_eq!(out[1].seq, 1);
}

#[test]
fn bootstrap_emits_empty_initial_values() {
    let meta = meta_map();
    let status = meta_status();

    // Snapshot: labels non-empty, status empty.
    let snapshot = snap(
        HashMap::from([(
            "labels".to_string(),
            FieldValue {
                ids: vec!["backend".into()],
                displays: vec!["backend".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    // Changelog: status set from None to "1" → initial state for status = empty, but still emitted.
    let events = vec![ev("cl-1", ts(2026, 1, 2, 9), &status, set(None, Some("1")))];

    let out = process_issue(&meta, &snapshot, &events, None);
    // 1 creation marker + 2 synthetic_initial (labels=[backend], status=empty)
    // + 1 changelog for status = 4 rows.
    assert_eq!(out.len(), 4);
    assert_eq!(out[0].field_id, "created");
    assert_eq!(out[0].seq, 0);
    assert_eq!(out[1].field_id, "labels");
    assert_eq!(out[1].seq, 1);
    assert_eq!(out[1].value_displays, vec!["backend".to_string()]);
    assert_eq!(out[2].field_id, "status");
    assert_eq!(out[2].seq, 2);
    assert!(out[2].value_ids.is_empty()); // status empty at creation
    assert_eq!(out[3].event_id, "cl-1");
    assert_eq!(out[3].event_kind, EventKind::Changelog);
    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[1].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[2].event_kind, EventKind::SyntheticInitial);
}

// ---------------- process_issue incremental ----------------

#[test]
fn incremental_emits_only_new_events() {
    let meta = meta_map();
    let status = meta_status();

    let snapshot = snap(
        HashMap::from([(
            "status".to_string(),
            FieldValue {
                ids: vec!["3".into()],
                displays: vec!["Done".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    // Full changelog we know about.
    let events = vec![
        ev(
            "cl-1",
            ts(2026, 1, 2, 9),
            &status,
            set(Some("1"), Some("2")),
        ),
        ev(
            "cl-2",
            ts(2026, 1, 3, 9),
            &status,
            set(Some("2"), Some("3")),
        ),
        ev(
            "cl-3",
            ts(2026, 1, 4, 9),
            &status,
            set(Some("3"), Some("2")),
        ), // reopened
        ev(
            "cl-4",
            ts(2026, 1, 5, 9),
            &status,
            set(Some("2"), Some("3")),
        ),
    ];

    // Already processed up to cl-2 at 2026-01-03 09:00.
    let existing = HashMap::from([(
        "status".to_string(),
        LastState {
            value: FieldValue {
                ids: vec!["3".into()],
                displays: vec!["Done".into()],
            },
            last_event_at: ts(2026, 1, 3, 9),
        },
    )]);

    let out = process_issue(&meta, &snapshot, &events, Some(&existing));

    // Only cl-3 and cl-4 should be emitted.
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].event_id, "cl-3");
    assert_eq!(out[0].value_ids, vec!["2".to_string()]);
    assert_eq!(out[1].event_id, "cl-4");
    assert_eq!(out[1].value_ids, vec!["3".to_string()]);
    for row in &out {
        assert_eq!(row.event_kind, EventKind::Changelog);
    }
}

#[test]
fn incremental_no_new_events_produces_empty() {
    let meta = meta_map();
    let status = meta_status();

    let snapshot = snap(
        HashMap::from([(
            "status".to_string(),
            FieldValue {
                ids: vec!["3".into()],
                displays: vec!["Done".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    let events = vec![ev(
        "cl-1",
        ts(2026, 1, 2, 9),
        &status,
        set(Some("1"), Some("3")),
    )];

    let existing = HashMap::from([(
        "status".to_string(),
        LastState {
            value: FieldValue {
                ids: vec!["3".into()],
                displays: vec!["Done".into()],
            },
            last_event_at: ts(2026, 1, 2, 9),
        },
    )]);

    let out = process_issue(&meta, &snapshot, &events, Some(&existing));
    assert!(out.is_empty());
}

#[test]
fn incremental_drops_late_events_below_hwm() {
    // Documented limitation from ADR-004: events with event_at <= per-issue HWM are silently dropped.
    let meta = meta_map();
    let status = meta_status();

    let snapshot = snap(HashMap::new(), ts(2026, 1, 1, 10));
    let late_event = ev("cl-late", ts(2026, 1, 2, 8), &status, set(None, Some("2")));
    let existing = HashMap::from([(
        "status".to_string(),
        LastState {
            value: FieldValue {
                ids: vec!["3".into()],
                displays: vec!["Done".into()],
            },
            last_event_at: ts(2026, 1, 3, 9), // HWM is AFTER the "late" event
        },
    )]);

    let out = process_issue(&meta, &snapshot, &[late_event], Some(&existing));
    assert!(
        out.is_empty(),
        "late event (event_at < HWM) must be dropped — see ADR-004"
    );
}

// ---------------- snapshot / sprint ----------------

#[test]
fn sprint_snapshot_delta_replaces_full_value() {
    let meta = meta_map();
    let sprint = meta_sprint();

    let snapshot = snap(
        HashMap::from([(
            "customfield_10020".to_string(),
            FieldValue {
                ids: vec!["24".into(), "25".into()],
                displays: vec!["Sprint 24".into(), "Sprint 25".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    let events = vec![ev(
        "cl-1",
        ts(2026, 1, 2, 9),
        &sprint,
        Delta::Snapshot {
            from_ids: vec!["24".into()],
            from_displays: vec!["Sprint 24".into()],
            to_ids: vec!["24".into(), "25".into()],
            to_displays: vec!["Sprint 24".into(), "Sprint 25".into()],
        },
    )];

    let out = process_issue(&meta, &snapshot, &events, None);
    // 1 creation marker + 1 initial (Sprint 24) + 1 changelog (Sprint 24, Sprint 25)
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[0].field_id, "created");
    assert_eq!(out[1].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[1].field_id, "customfield_10020");
    assert_eq!(out[1].value_ids, vec!["24".to_string()]);
    assert_eq!(out[2].event_kind, EventKind::Changelog);
    assert_eq!(out[2].value_ids, vec!["24".to_string(), "25".to_string()]);
}

// ---------------- unknown field handling ----------------

#[test]
fn unknown_field_is_skipped_with_warning() {
    let meta = meta_map();
    let unknown = FieldMeta {
        field_id: "customfield_99999".into(),
        field_name: "Unknown".into(),
        cardinality: FieldCardinality::Single,
        value_id_type: ValueIdType::None,
    };

    let snapshot = snap(HashMap::new(), ts(2026, 1, 1, 10));
    let events = vec![ev(
        "cl-1",
        ts(2026, 1, 2, 9),
        &unknown,
        set(None, Some("x")),
    )];

    let out = process_issue(&meta, &snapshot, &events, None);
    // Empty snapshot + only an unknown-field event → just the creation marker; the unknown
    // field produces no synthetic_initial and no changelog row.
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].field_id, "created");
    assert!(
        out.iter().all(|r| r.field_id != "customfield_99999"),
        "no row should reference the unknown field"
    );
}

#[test]
fn data_source_is_always_jira() {
    let meta = meta_map();
    let snapshot = snap(
        HashMap::from([(
            "status".to_string(),
            FieldValue {
                ids: vec!["1".into()],
                displays: vec!["To Do".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );
    let out = process_issue(&meta, &snapshot, &[], None);
    // 1 creation marker + 1 synthetic_initial for status.
    assert_eq!(out.len(), 2);
    for row in &out {
        assert_eq!(row.data_source, DataSource::Jira);
    }
}

// ---------------- apply/reverse matrix completion ----------------

#[test]
fn apply_single_add_sets_id_and_display() {
    // Single-cardinality Add behaves as a replace: ids=[id], displays=[display].
    let out = apply_delta(
        FieldValue {
            ids: vec!["old".into()],
            displays: vec!["Old".into()],
        },
        &Delta::Add {
            id: "new".into(),
            display: "New".into(),
        },
        FieldCardinality::Single,
    );
    assert_eq!(out.ids, vec!["new".to_string()]);
    assert_eq!(out.displays, vec!["New".to_string()]);
}

#[test]
fn apply_single_remove_clears_value() {
    let out = apply_delta(
        FieldValue {
            ids: vec!["x".into()],
            displays: vec!["X".into()],
        },
        &Delta::Remove {
            id: "x".into(),
            display: "X".into(),
        },
        FieldCardinality::Single,
    );
    assert!(out.ids.is_empty());
    assert!(out.displays.is_empty());
}

#[test]
fn reverse_single_add_and_remove_are_empty() {
    // For Single cardinality, both Add and Remove reverse to empty (no from-side info).
    let after_add = reverse_delta(
        FieldValue {
            ids: vec!["x".into()],
            displays: vec!["X".into()],
        },
        &Delta::Add {
            id: "x".into(),
            display: "X".into(),
        },
        FieldCardinality::Single,
    );
    assert!(after_add.ids.is_empty());
    assert!(after_add.displays.is_empty());

    let after_remove = reverse_delta(
        FieldValue {
            ids: vec!["y".into()],
            displays: vec!["Y".into()],
        },
        &Delta::Remove {
            id: "y".into(),
            display: "Y".into(),
        },
        FieldCardinality::Single,
    );
    assert!(after_remove.ids.is_empty());
    assert!(after_remove.displays.is_empty());
}

#[test]
fn reverse_snapshot_uses_from_side() {
    let before = reverse_delta(
        FieldValue {
            ids: vec!["24".into(), "25".into()],
            displays: vec!["Sprint 24".into(), "Sprint 25".into()],
        },
        &Delta::Snapshot {
            from_ids: vec!["24".into()],
            from_displays: vec!["Sprint 24".into()],
            to_ids: vec!["24".into(), "25".into()],
            to_displays: vec!["Sprint 24".into(), "Sprint 25".into()],
        },
        FieldCardinality::Multi,
    );
    assert_eq!(before.ids, vec!["24".to_string()]);
    assert_eq!(before.displays, vec!["Sprint 24".to_string()]);
}

#[test]
fn apply_and_reverse_set_id_only_uses_id_as_display() {
    // to_display=None → display falls back to the id.
    let applied = apply_delta(
        FieldValue {
            ids: vec!["old".into()],
            displays: vec!["old".into()],
        },
        &Delta::Set {
            from: Some("old".into()),
            from_display: None,
            to: Some("7".into()),
            to_display: None,
        },
        FieldCardinality::Single,
    );
    assert_eq!(applied.ids, vec!["7".to_string()]);
    assert_eq!(applied.displays, vec!["7".to_string()]);

    // from_display=None → display falls back to the from id.
    let reversed = reverse_delta(
        FieldValue {
            ids: vec!["7".into()],
            displays: vec!["7".into()],
        },
        &Delta::Set {
            from: Some("old".into()),
            from_display: None,
            to: Some("7".into()),
            to_display: None,
        },
        FieldCardinality::Single,
    );
    assert_eq!(reversed.ids, vec!["old".to_string()]);
    assert_eq!(reversed.displays, vec!["old".to_string()]);
}

#[test]
fn reverse_set_from_none_clears_value() {
    let before = reverse_delta(
        FieldValue {
            ids: vec!["2".into()],
            displays: vec!["In Progress".into()],
        },
        &set(None, Some("2")),
        FieldCardinality::Single,
    );
    assert!(before.ids.is_empty());
    assert!(before.displays.is_empty());
}

// ---------------- round-trip invariants ----------------
//
// reverse_delta(apply_delta(state, Δ, card), Δ, card) == state holds only when Δ
// actually changes state. We deliberately do NOT test Add of an already-present id
// or Remove of an absent id here — those are no-ops by design (dedup / no-match), so
// they trivially round-trip but exercise nothing.

#[test]
fn roundtrip_set_single() {
    let state = FieldValue {
        ids: vec!["1".into()],
        displays: vec!["To Do".into()],
    };
    let delta = set_full(Some(("1", "To Do")), Some(("3", "Done")));
    let after = apply_delta(state.clone(), &delta, FieldCardinality::Single);
    assert_eq!(after.ids, vec!["3".to_string()]); // sanity: state actually changed
    let back = reverse_delta(after, &delta, FieldCardinality::Single);
    assert_eq!(back, state);
}

#[test]
fn roundtrip_snapshot_multi() {
    let state = FieldValue {
        ids: vec!["24".into()],
        displays: vec!["Sprint 24".into()],
    };
    let delta = Delta::Snapshot {
        from_ids: vec!["24".into()],
        from_displays: vec!["Sprint 24".into()],
        to_ids: vec!["24".into(), "25".into()],
        to_displays: vec!["Sprint 24".into(), "Sprint 25".into()],
    };
    let after = apply_delta(state.clone(), &delta, FieldCardinality::Multi);
    assert_eq!(after.ids, vec!["24".to_string(), "25".to_string()]); // changed
    let back = reverse_delta(after, &delta, FieldCardinality::Multi);
    assert_eq!(back, state);
}

#[test]
fn roundtrip_multi_add_new_id() {
    let state = FieldValue {
        ids: vec!["urgent".into()],
        displays: vec!["urgent".into()],
    };
    let delta = Delta::Add {
        id: "backend".into(),
        display: "backend".into(),
    };
    let after = apply_delta(state.clone(), &delta, FieldCardinality::Multi);
    assert_eq!(after.ids, vec!["urgent".to_string(), "backend".to_string()]); // changed
    let back = reverse_delta(after, &delta, FieldCardinality::Multi);
    assert_eq!(back, state);
}

#[test]
fn roundtrip_multi_remove_present_id() {
    let state = FieldValue {
        ids: vec!["urgent".into(), "backend".into()],
        displays: vec!["urgent".into(), "backend".into()],
    };
    let delta = Delta::Remove {
        id: "backend".into(),
        display: "backend".into(),
    };
    let after = apply_delta(state.clone(), &delta, FieldCardinality::Multi);
    assert_eq!(after.ids, vec!["urgent".to_string()]); // changed
    let back = reverse_delta(after, &delta, FieldCardinality::Multi);
    assert_eq!(back, state);
}

#[test]
fn full_issue_last_changelog_per_field_matches_snapshot() {
    // Forward-applying the reconstructed initial state through the whole changelog must
    // reproduce the snapshot's current value for every field that the changelog touched.
    let meta = meta_map();
    let status = meta_status();
    let labels = meta_labels();

    let snapshot = snap(
        HashMap::from([
            (
                "status".to_string(),
                FieldValue {
                    ids: vec!["3".into()],
                    displays: vec!["Done".into()],
                },
            ),
            (
                "labels".to_string(),
                FieldValue {
                    ids: vec!["backend".into(), "urgent".into()],
                    displays: vec!["backend".into(), "urgent".into()],
                },
            ),
        ]),
        ts(2026, 1, 1, 10),
    );

    let events = vec![
        ev(
            "cl-1",
            ts(2026, 1, 2, 9),
            &status,
            set_full(Some(("1", "To Do")), Some(("2", "In Progress"))),
        ),
        ev(
            "cl-2",
            ts(2026, 1, 3, 9),
            &labels,
            Delta::Add {
                id: "backend".into(),
                display: "backend".into(),
            },
        ),
        ev(
            "cl-3",
            ts(2026, 1, 4, 9),
            &status,
            set_full(Some(("2", "In Progress")), Some(("3", "Done"))),
        ),
        ev(
            "cl-4",
            ts(2026, 1, 5, 9),
            &labels,
            Delta::Add {
                id: "urgent".into(),
                display: "urgent".into(),
            },
        ),
    ];

    let out = process_issue(&meta, &snapshot, &events, None);

    // Last changelog row per field carries the running state-after, which for the final
    // event of each field must equal the snapshot's current value.
    let last_status = out
        .iter()
        .rfind(|r| r.field_id == "status" && r.event_kind == EventKind::Changelog)
        .unwrap();
    assert_eq!(last_status.value_ids, vec!["3".to_string()]);
    assert_eq!(last_status.value_displays, vec!["Done".to_string()]);

    let last_labels = out
        .iter()
        .rfind(|r| r.field_id == "labels" && r.event_kind == EventKind::Changelog)
        .unwrap();
    assert_eq!(
        last_labels.value_ids,
        vec!["backend".to_string(), "urgent".to_string()]
    );
    assert_eq!(
        last_labels.value_displays,
        vec!["backend".to_string(), "urgent".to_string()]
    );
}

// ---------------- bootstrap scenarios ----------------

#[test]
fn bootstrap_status_reopen_toggle() {
    let meta = meta_map();
    let status = meta_status();

    // Final state: Done (3).
    let snapshot = snap(
        HashMap::from([(
            "status".to_string(),
            FieldValue {
                ids: vec!["3".into()],
                displays: vec!["Done".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    // To Do(1)→In Progress(2)→Done(3)→In Progress(2) [reopen]→Done(3)
    let events = vec![
        ev(
            "cl-1",
            ts(2026, 1, 2, 9),
            &status,
            set_full(Some(("1", "To Do")), Some(("2", "In Progress"))),
        ),
        ev(
            "cl-2",
            ts(2026, 1, 3, 9),
            &status,
            set_full(Some(("2", "In Progress")), Some(("3", "Done"))),
        ),
        ev(
            "cl-3",
            ts(2026, 1, 4, 9),
            &status,
            set_full(Some(("3", "Done")), Some(("2", "In Progress"))),
        ),
        ev(
            "cl-4",
            ts(2026, 1, 5, 9),
            &status,
            set_full(Some(("2", "In Progress")), Some(("3", "Done"))),
        ),
    ];

    let out = process_issue(&meta, &snapshot, &events, None);
    // 1 creation marker + 1 synthetic_initial + 4 changelog.
    assert_eq!(out.len(), 6);

    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[0].field_id, "created");
    assert_eq!(out[0].seq, 0);

    assert_eq!(out[1].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[1].field_id, "status");
    assert_eq!(out[1].seq, 1);
    assert_eq!(out[1].value_ids, vec!["1".to_string()]); // reconstructed initial = To Do
    assert_eq!(out[1].value_displays, vec!["To Do".to_string()]);

    let expected = [
        ("cl-1", "2", "In Progress"),
        ("cl-2", "3", "Done"),
        ("cl-3", "2", "In Progress"),
        ("cl-4", "3", "Done"),
    ];
    for (i, (event_id, id, disp)) in expected.iter().enumerate() {
        let row = &out[i + 2];
        assert_eq!(row.event_id, *event_id);
        assert_eq!(row.event_kind, EventKind::Changelog);
        assert_eq!(row.value_ids, vec![(*id).to_string()]);
        assert_eq!(row.value_displays, vec![(*disp).to_string()]);
    }
}

#[test]
fn bootstrap_multi_add_remove_interplay() {
    let meta = meta_map();
    let labels = meta_labels();

    // Final state: [urgent].
    let snapshot = snap(
        HashMap::from([(
            "labels".to_string(),
            FieldValue {
                ids: vec!["urgent".into()],
                displays: vec!["urgent".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    // Add backend, Add urgent, Remove backend.
    let events = vec![
        ev(
            "cl-1",
            ts(2026, 1, 2, 9),
            &labels,
            Delta::Add {
                id: "backend".into(),
                display: "backend".into(),
            },
        ),
        ev(
            "cl-2",
            ts(2026, 1, 3, 9),
            &labels,
            Delta::Add {
                id: "urgent".into(),
                display: "urgent".into(),
            },
        ),
        ev(
            "cl-3",
            ts(2026, 1, 4, 9),
            &labels,
            Delta::Remove {
                id: "backend".into(),
                display: "backend".into(),
            },
        ),
    ];

    let out = process_issue(&meta, &snapshot, &events, None);
    assert_eq!(out.len(), 5); // 1 creation marker + 1 initial + 3 changelog

    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[0].field_id, "created");

    assert_eq!(out[1].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[1].field_id, "labels");
    assert!(out[1].value_ids.is_empty()); // reconstructed initial = []
    assert!(out[1].value_displays.is_empty());

    assert_eq!(out[2].event_id, "cl-1");
    assert_eq!(out[2].value_ids, vec!["backend".to_string()]);
    assert_eq!(out[2].value_displays, vec!["backend".to_string()]);

    assert_eq!(out[3].event_id, "cl-2");
    assert_eq!(
        out[3].value_ids,
        vec!["backend".to_string(), "urgent".to_string()]
    );
    assert_eq!(
        out[3].value_displays,
        vec!["backend".to_string(), "urgent".to_string()]
    );

    assert_eq!(out[4].event_id, "cl-3");
    assert_eq!(out[4].value_ids, vec!["urgent".to_string()]);
    assert_eq!(out[4].value_displays, vec!["urgent".to_string()]);
}

#[test]
fn bootstrap_mixed_known_and_unknown_field() {
    let meta = meta_map();
    let status = meta_status();
    // Unknown: field_id not present in meta_map().
    let unknown = FieldMeta {
        field_id: "customfield_99999".into(),
        field_name: "Unknown".into(),
        cardinality: FieldCardinality::Single,
        value_id_type: ValueIdType::None,
    };

    // Snapshot has only status; the unknown field is purely in the changelog.
    let snapshot = snap(
        HashMap::from([(
            "status".to_string(),
            FieldValue {
                ids: vec!["2".into()],
                displays: vec!["In Progress".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    let events = vec![
        ev(
            "cl-known",
            ts(2026, 1, 2, 9),
            &status,
            set_full(Some(("1", "To Do")), Some(("2", "In Progress"))),
        ),
        ev(
            "cl-unknown",
            ts(2026, 1, 3, 9),
            &unknown,
            set(Some("a"), Some("b")),
        ),
    ];

    let out = process_issue(&meta, &snapshot, &events, None);

    // 1 creation marker + 1 synthetic_initial (status) + 1 changelog (status).
    // Unknown field produces NO rows.
    assert_eq!(out.len(), 3);
    assert!(
        out.iter()
            .all(|r| r.field_id == "status" || r.field_id == "created"),
        "no row should reference the unknown field"
    );

    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[0].field_id, "created");

    assert_eq!(out[1].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[1].field_id, "status");
    assert_eq!(out[1].value_ids, vec!["1".to_string()]); // initial = To Do

    assert_eq!(out[2].event_id, "cl-known");
    assert_eq!(out[2].value_ids, vec!["2".to_string()]);
    assert_eq!(out[2].value_displays, vec!["In Progress".to_string()]);
}

#[test]
fn bootstrap_meta_absent_field_infers_multi_cardinality() {
    // A field present in the snapshot but with NO meta entry: cardinality is inferred from
    // shape (Multi when >1 id), value_id_type is None, delta_action is Add (Multi).
    let meta = meta_map();
    let snapshot = snap(
        HashMap::from([(
            "weirdfield".to_string(),
            FieldValue {
                ids: vec!["a".into(), "b".into()],
                displays: vec!["Alpha".into(), "Beta".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    let out = process_issue(&meta, &snapshot, &[], None);
    // 1 creation marker + 1 synthetic_initial for the meta-absent field.
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].field_id, "created");
    let row = &out[1];
    assert_eq!(row.event_kind, EventKind::SyntheticInitial);
    assert_eq!(row.field_id, "weirdfield");
    assert_eq!(row.seq, 1);
    assert_eq!(row.field_cardinality, FieldCardinality::Multi);
    assert_eq!(row.value_id_type, ValueIdType::None);
    assert_eq!(row.delta_action, DeltaAction::Add);
    assert_eq!(row.value_ids, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(
        row.value_displays,
        vec!["Alpha".to_string(), "Beta".to_string()]
    );
}

#[test]
fn synthetic_initial_metadata_is_correct() {
    let meta = meta_map();
    // status = Single, labels = Multi. Both present in the snapshot, empty changelog.
    let snapshot = snap(
        HashMap::from([
            (
                "status".to_string(),
                FieldValue {
                    ids: vec!["1".into()],
                    displays: vec!["To Do".into()],
                },
            ),
            (
                "labels".to_string(),
                FieldValue {
                    ids: vec!["backend".into()],
                    displays: vec!["backend".into()],
                },
            ),
        ]),
        ts(2026, 1, 1, 10),
    );

    let out = process_issue(&meta, &snapshot, &[], None);
    // 1 creation marker + 2 per-field synthetic_initial rows.
    assert_eq!(out.len(), 3);

    for row in &out {
        assert_eq!(row.event_kind, EventKind::SyntheticInitial);
        assert_eq!(row.author_id, Some("acc-1".to_string())); // snapshot.reporter_id
        assert_eq!(row.event_at, ts(2026, 1, 1, 10)); // snapshot.created_at
        assert_eq!(row.event_id, "initial:10042");
    }

    // The creation marker is first.
    assert_eq!(out[0].field_id, "created");
    assert_eq!(out[0].seq, 0);

    // Sorted by field_id ASC: labels (Multi) before status (Single).
    let labels_row = out.iter().find(|r| r.field_id == "labels").unwrap();
    assert_eq!(labels_row.field_cardinality, FieldCardinality::Multi);
    assert_eq!(labels_row.delta_action, DeltaAction::Add);

    let status_row = out.iter().find(|r| r.field_id == "status").unwrap();
    assert_eq!(status_row.field_cardinality, FieldCardinality::Single);
    assert_eq!(status_row.delta_action, DeltaAction::Set);
}

// ---------------- incremental edges ----------------

#[test]
fn incremental_event_at_hwm_is_dropped_strictly_after_is_kept() {
    // The <= hwm rule: an event exactly AT the hwm is dropped; strictly after is kept.
    let meta = meta_map();
    let status = meta_status();

    let snapshot = snap(HashMap::new(), ts(2026, 1, 1, 10));

    let hwm = ts(2026, 1, 3, 9);
    let events = vec![
        ev("cl-at-hwm", hwm, &status, set(Some("2"), Some("3"))),
        ev(
            "cl-after-hwm",
            ts(2026, 1, 4, 9),
            &status,
            set(Some("3"), Some("4")),
        ),
    ];

    let existing = HashMap::from([(
        "status".to_string(),
        LastState {
            value: FieldValue {
                ids: vec!["3".into()],
                displays: vec!["Done".into()],
            },
            last_event_at: hwm,
        },
    )]);

    let out = process_issue(&meta, &snapshot, &events, Some(&existing));
    assert_eq!(out.len(), 1, "only the strictly-after event is kept");
    assert_eq!(out[0].event_id, "cl-after-hwm");
    // Forward-applied from existing state ("3") through Set(3→4).
    assert_eq!(out[0].value_ids, vec!["4".to_string()]);
    assert_eq!(out[0].value_displays, vec!["4".to_string()]);
}

#[test]
fn incremental_new_field_first_seen_after_hwm_starts_empty() {
    // A field not present in `existing` is forward-applied from empty.
    let meta = meta_map();
    let labels = meta_labels();

    let snapshot = snap(HashMap::new(), ts(2026, 1, 1, 10));

    let hwm = ts(2026, 1, 3, 9);
    // labels event is strictly after hwm and the field is absent from `existing`.
    let events = vec![ev(
        "cl-labels",
        ts(2026, 1, 4, 9),
        &labels,
        Delta::Add {
            id: "backend".into(),
            display: "backend".into(),
        },
    )];

    let existing = HashMap::from([(
        "status".to_string(),
        LastState {
            value: FieldValue {
                ids: vec!["3".into()],
                displays: vec!["Done".into()],
            },
            last_event_at: hwm,
        },
    )]);

    let out = process_issue(&meta, &snapshot, &events, Some(&existing));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].event_id, "cl-labels");
    assert_eq!(out[0].field_id, "labels");
    assert_eq!(out[0].event_kind, EventKind::Changelog);
    // Started from empty (labels not in existing) then Add backend.
    assert_eq!(out[0].value_ids, vec!["backend".to_string()]);
    assert_eq!(out[0].value_displays, vec!["backend".to_string()]);
}

// ---------------- creation marker ----------------

#[test]
fn bootstrap_emits_creation_marker_first() {
    let meta = meta_map();
    let snapshot = snap(
        HashMap::from([(
            "status".to_string(),
            FieldValue {
                ids: vec!["1".into()],
                displays: vec!["To Do".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    let out = process_issue(&meta, &snapshot, &[], None);

    let marker = &out[0];
    assert_eq!(marker.field_id, "created");
    assert_eq!(marker.field_name, "Created");
    assert_eq!(marker.event_kind, EventKind::SyntheticInitial);
    assert_eq!(marker.seq, 0);
    assert_eq!(marker.event_id, "initial:10042");
    assert_eq!(marker.event_at, ts(2026, 1, 1, 10)); // snapshot.created_at
    assert_eq!(marker.author_id, Some("acc-1".to_string())); // snapshot.reporter_id
    assert_eq!(marker.author_display, None);
    assert!(marker.value_ids.is_empty());
    assert!(marker.value_displays.is_empty());
    assert_eq!(marker.delta_value_id, None);
    assert_eq!(marker.delta_value_display, None);
    assert_eq!(marker.delta_action, DeltaAction::Set);
    assert_eq!(marker.value_id_type, ValueIdType::None);
    assert_eq!(marker.field_cardinality, FieldCardinality::Single);
    assert_eq!(marker.data_source, DataSource::Jira);
}

#[test]
fn bootstrap_creation_marker_present_with_empty_changelog() {
    let meta = meta_map();
    let snapshot = snap(
        HashMap::from([(
            "status".to_string(),
            FieldValue {
                ids: vec!["1".into()],
                displays: vec!["To Do".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    let out = process_issue(&meta, &snapshot, &[], None);

    // Exactly 2 rows: creation marker (seq 0) then the single field (seq 1).
    assert_eq!(out.len(), 2);

    assert_eq!(out[0].field_id, "created");
    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[0].seq, 0);

    assert_eq!(out[1].field_id, "status");
    assert_eq!(out[1].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[1].seq, 1);
}

#[test]
fn bootstrap_stable_order_three_fields() {
    let meta = meta_map();
    let status = meta_status();

    // Three fields in the snapshot: status, labels, customfield_10020 (sprint).
    let snapshot = snap(
        HashMap::from([
            (
                "status".to_string(),
                FieldValue {
                    ids: vec!["1".into()],
                    displays: vec!["To Do".into()],
                },
            ),
            (
                "labels".to_string(),
                FieldValue {
                    ids: vec!["backend".into()],
                    displays: vec!["backend".into()],
                },
            ),
            (
                "customfield_10020".to_string(),
                FieldValue {
                    ids: vec!["24".into()],
                    displays: vec!["Sprint 24".into()],
                },
            ),
        ]),
        ts(2026, 1, 1, 10),
    );

    // A small changelog so we can also assert synthetic-before-changelog ordering.
    let events = vec![ev(
        "cl-1",
        ts(2026, 1, 2, 9),
        &status,
        set_full(Some(("1", "To Do")), Some(("2", "In Progress"))),
    )];

    let out = process_issue(&meta, &snapshot, &events, None);

    // out[0] = creation marker (seq 0).
    assert_eq!(out[0].field_id, "created");
    assert_eq!(out[0].seq, 0);
    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);

    // out[1..=3] = the three fields in field_id-ASC order, with CONTIGUOUS seq 1,2,3.
    // ASC: customfield_10020 < labels < status.
    let expected_fields = ["customfield_10020", "labels", "status"];
    for (i, field_id) in expected_fields.iter().enumerate() {
        let row = &out[i + 1];
        assert_eq!(row.field_id, *field_id);
        assert_eq!(row.seq, u32::try_from(i + 1).unwrap());
        assert_eq!(row.event_kind, EventKind::SyntheticInitial);
    }

    // Every synthetic row precedes every changelog row.
    let last_synthetic = out
        .iter()
        .rposition(|r| r.event_kind == EventKind::SyntheticInitial)
        .unwrap();
    let first_changelog = out
        .iter()
        .position(|r| r.event_kind == EventKind::Changelog)
        .unwrap();
    assert!(
        last_synthetic < first_changelog,
        "all synthetic rows must precede all changelog rows"
    );
}

#[test]
fn incremental_emits_no_creation_marker() {
    let meta = meta_map();
    let status = meta_status();

    let snapshot = snap(
        HashMap::from([(
            "status".to_string(),
            FieldValue {
                ids: vec!["3".into()],
                displays: vec!["Done".into()],
            },
        )]),
        ts(2026, 1, 1, 10),
    );

    let events = vec![
        ev(
            "cl-1",
            ts(2026, 1, 2, 9),
            &status,
            set(Some("1"), Some("2")),
        ),
        ev(
            "cl-2",
            ts(2026, 1, 3, 9),
            &status,
            set(Some("2"), Some("3")),
        ),
    ];

    let existing = HashMap::from([(
        "status".to_string(),
        LastState {
            value: FieldValue {
                ids: vec!["1".into()],
                displays: vec!["To Do".into()],
            },
            last_event_at: ts(2026, 1, 1, 12),
        },
    )]);

    let out = process_issue(&meta, &snapshot, &events, Some(&existing));

    assert!(!out.is_empty());
    assert!(
        out.iter().all(|r| r.field_id != "created"),
        "incremental must not emit the creation marker"
    );
    assert!(
        out.iter().all(|r| r.event_kind == EventKind::Changelog),
        "incremental must emit only changelog rows"
    );
}
