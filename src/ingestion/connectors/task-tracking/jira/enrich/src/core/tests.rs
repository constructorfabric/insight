use super::types::{
    DataSource, Delta, DeltaAction, DeltaEvent, EventKind, FieldCardinality, FieldMeta, FieldValue,
    IssueSnapshot, LastState, ValueIdType, synthetic_initial_event_id,
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

fn ev(
    event_id: &str,
    event_at: DateTime<Utc>,
    field: &FieldMeta,
    delta: Delta,
) -> DeltaEvent {
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
    let out = apply_delta(initial, &set(Some("1"), Some("3")), FieldCardinality::Single);
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
    assert_eq!(second.ids, vec!["urgent".to_string(), "backend".to_string()]);
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
    assert_eq!(before.ids, vec!["urgent".to_string(), "backend".to_string()]);
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

    // 2 synthetic_initial rows (labels=empty sorted first by field_id, status=To Do) + 4 changelog.
    assert_eq!(out.len(), 6);

    // synthetic_initial rows are sorted by field_id ASC → "labels" comes before "status".
    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[0].field_id, "labels");
    assert_eq!(out[0].seq, 0);
    assert!(out[0].value_displays.is_empty()); // empty labels at creation
    assert_eq!(out[0].event_id, "initial:10042");

    assert_eq!(out[1].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[1].field_id, "status");
    assert_eq!(out[1].seq, 1);
    assert_eq!(out[1].value_displays, vec!["To Do".to_string()]);

    // Changelog events follow, in chronological order (seq=0 for all changelog).
    assert_eq!(out[2].event_id, "cl-1");
    assert_eq!(out[2].value_ids, vec!["2".to_string()]);
    assert_eq!(out[2].seq, 0);

    assert_eq!(out[3].event_id, "cl-2");
    assert_eq!(out[3].delta_action, DeltaAction::Add);
    assert_eq!(out[3].value_ids, vec!["backend".to_string()]);

    assert_eq!(out[4].event_id, "cl-3");
    assert_eq!(out[4].value_ids, vec!["3".to_string()]);

    assert_eq!(out[5].event_id, "cl-4");
    assert_eq!(
        out[5].value_ids,
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
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[0].field_id, "status");
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
    // 2 synthetic_initial (labels=[backend], status=empty) + 1 changelog for status = 3 rows.
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].field_id, "labels");
    assert_eq!(out[0].seq, 0);
    assert_eq!(out[0].value_displays, vec!["backend".to_string()]);
    assert_eq!(out[1].field_id, "status");
    assert_eq!(out[1].seq, 1);
    assert!(out[1].value_ids.is_empty()); // status empty at creation
    assert_eq!(out[2].event_id, "cl-1");
    assert_eq!(out[2].event_kind, EventKind::Changelog);
    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[1].event_kind, EventKind::SyntheticInitial);
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
        ev("cl-1", ts(2026, 1, 2, 9), &status, set(Some("1"), Some("2"))),
        ev("cl-2", ts(2026, 1, 3, 9), &status, set(Some("2"), Some("3"))),
        ev("cl-3", ts(2026, 1, 4, 9), &status, set(Some("3"), Some("2"))), // reopened
        ev("cl-4", ts(2026, 1, 5, 9), &status, set(Some("2"), Some("3"))),
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
    let late_event = ev(
        "cl-late",
        ts(2026, 1, 2, 8),
        &status,
        set(None, Some("2")),
    );
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
    // 1 initial (Sprint 24) + 1 changelog (Sprint 24, Sprint 25)
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].event_kind, EventKind::SyntheticInitial);
    assert_eq!(out[0].value_ids, vec!["24".to_string()]);
    assert_eq!(out[1].event_kind, EventKind::Changelog);
    assert_eq!(
        out[1].value_ids,
        vec!["24".to_string(), "25".to_string()]
    );
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
    assert!(out.is_empty());
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
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].data_source, DataSource::Jira);
}
