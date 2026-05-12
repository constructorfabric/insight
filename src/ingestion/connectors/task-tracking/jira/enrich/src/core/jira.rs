//! Jira-specific converter: `bronze_jira.jira_issue_history` row → `Delta`.
//!
//! Jira emits changelog items in three distinct shapes (see docs/community threads):
//!
//! 1. **Single-value field** — one item, both `from`/`to` present (or one NULL).
//!    → `Delta::Set`.
//!
//! 2. **Native multi-value field** (Components, Fix Versions, Labels, Linked Issues,
//!    Attachment, …) — **one item per element change**. `fromString=NULL,toString=X`
//!    means add; `fromString=X,toString=NULL` means remove. Multiple items share the
//!    same `changelog_id` when they happen in one action.
//!    → `Delta::Add` / `Delta::Remove`.
//!
//! 3. **Legacy custom multi-value field** (Sprint via `customfield_*`, Roadmap-like
//!    quarter fields, some third-party plugins) — **one item with BOTH sides as a
//!    comma-separated full list**. E.g. `fromString="A, B"`, `toString="A, B, C"`.
//!    This comes from pre-2020 Sprint serialization which Atlassian never removed
//!    from the changelog API.
//!    → `Delta::Snapshot` with parsed lists.
//!
//! Detection of shape (3): for a multi-value field, if **either** `fromString` or
//! `toString` contains `, ` (Jira always uses `", "` — comma + space), treat the row
//! as a Snapshot. Parse both sides by splitting on `", "`.

use super::types::{Delta, FieldCardinality};

/// Raw per-row bronze payload needed to produce a `Delta`.
#[derive(Debug, Clone)]
pub struct BronzeChangelogItem<'a> {
    pub field_id: &'a str,
    pub value_from: Option<&'a str>,
    pub value_from_string: Option<&'a str>,
    pub value_to: Option<&'a str>,
    pub value_to_string: Option<&'a str>,
}

const LIST_SEP: &str = ", ";

/// Convert one bronze row to a `Delta`.
/// Returns `None` when the row has neither side (degenerate / corrupt).
pub fn to_delta(row: &BronzeChangelogItem<'_>, cardinality: FieldCardinality) -> Option<Delta> {
    match cardinality {
        FieldCardinality::Single => Some(single_to_set(row)),
        FieldCardinality::Multi => {
            if is_legacy_list_format(row) {
                Some(multi_to_snapshot(row))
            } else {
                multi_to_add_or_remove(row)
            }
        }
    }
}

fn single_to_set(row: &BronzeChangelogItem<'_>) -> Delta {
    Delta::Set {
        from: row.value_from.map(str::to_owned),
        from_display: row.value_from_string.map(str::to_owned),
        to: row.value_to.map(str::to_owned),
        to_display: row.value_to_string.map(str::to_owned),
    }
}

/// Shape (3) detector: multi-value field where Jira emitted a legacy comma-list.
fn is_legacy_list_format(row: &BronzeChangelogItem<'_>) -> bool {
    contains_list(row.value_from_string) || contains_list(row.value_to_string)
}

fn contains_list(s: Option<&str>) -> bool {
    s.is_some_and(|s| s.contains(LIST_SEP))
}

fn multi_to_snapshot(row: &BronzeChangelogItem<'_>) -> Delta {
    let from_displays = parse_list(row.value_from_string);
    let to_displays = parse_list(row.value_to_string);
    // Jira puts numeric IDs in `from`/`to` in the same comma-list order — use if present,
    // otherwise fall back to displays (acts as string_literal IDs).
    let from_ids = parse_list(row.value_from)
        .into_iter()
        .chain(std::iter::repeat(String::new()))
        .zip(from_displays.iter())
        .map(|(id, disp)| if id.is_empty() { disp.clone() } else { id })
        .collect::<Vec<_>>();
    let to_ids = parse_list(row.value_to)
        .into_iter()
        .chain(std::iter::repeat(String::new()))
        .zip(to_displays.iter())
        .map(|(id, disp)| if id.is_empty() { disp.clone() } else { id })
        .collect::<Vec<_>>();
    Delta::Snapshot {
        from_ids,
        from_displays,
        to_ids,
        to_displays,
    }
}

fn parse_list(s: Option<&str>) -> Vec<String> {
    match s {
        None => Vec::new(),
        Some(s) if s.is_empty() => Vec::new(),
        Some(s) => s.split(LIST_SEP).map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect(),
    }
}

fn multi_to_add_or_remove(row: &BronzeChangelogItem<'_>) -> Option<Delta> {
    match (row.value_to, row.value_to_string) {
        (Some(id), Some(disp)) => Some(Delta::Add {
            id: id.to_owned(),
            display: disp.to_owned(),
        }),
        (Some(id), None) => Some(Delta::Add {
            id: id.to_owned(),
            display: id.to_owned(),
        }),
        (None, _) => match (row.value_from, row.value_from_string) {
            (Some(id), Some(disp)) => Some(Delta::Remove {
                id: id.to_owned(),
                display: disp.to_owned(),
            }),
            (Some(id), None) => Some(Delta::Remove {
                id: id.to_owned(),
                display: id.to_owned(),
            }),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::FieldCardinality;
    use super::{BronzeChangelogItem, Delta, to_delta};

    fn item<'a>(
        field_id: &'a str,
        value_from: Option<&'a str>,
        value_from_string: Option<&'a str>,
        value_to: Option<&'a str>,
        value_to_string: Option<&'a str>,
    ) -> BronzeChangelogItem<'a> {
        BronzeChangelogItem {
            field_id,
            value_from,
            value_from_string,
            value_to,
            value_to_string,
        }
    }

    #[test]
    fn single_value_row_becomes_set() {
        let row = item("status", Some("1"), Some("To Do"), Some("3"), Some("Done"));
        let delta = to_delta(&row, FieldCardinality::Single).unwrap();
        let Delta::Set { from, to, .. } = delta else {
            panic!("expected Set");
        };
        assert_eq!(from.as_deref(), Some("1"));
        assert_eq!(to.as_deref(), Some("3"));
    }

    #[test]
    fn single_value_with_comma_still_becomes_set() {
        // Defensive: Description, Summary etc. are single-value even though displays may have
        // commas. Cardinality drives the decision, not content.
        let row = item(
            "description",
            Some("a, b"),
            Some("a, b"),
            Some("c, d, e"),
            Some("c, d, e"),
        );
        let delta = to_delta(&row, FieldCardinality::Single).unwrap();
        assert!(matches!(delta, Delta::Set { .. }));
    }

    #[test]
    fn multi_value_native_add_one_item() {
        let row = item("labels", None, None, Some("backend"), Some("backend"));
        let delta = to_delta(&row, FieldCardinality::Multi).unwrap();
        let Delta::Add { id, display } = delta else {
            panic!("expected Add");
        };
        assert_eq!(id, "backend");
        assert_eq!(display, "backend");
    }

    #[test]
    fn multi_value_native_remove_one_item() {
        let row = item("labels", Some("urgent"), Some("urgent"), None, None);
        let delta = to_delta(&row, FieldCardinality::Multi).unwrap();
        assert!(matches!(delta, Delta::Remove { .. }));
    }

    #[test]
    fn sprint_legacy_list_becomes_snapshot() {
        // Legacy custom Sprint field: fromString = old list, toString = new list, both with ", ".
        let row = item(
            "customfield_10100",
            Some("8066, 8363"),
            Some("HCI - 26, HCI - 27"),
            Some("8066, 8363, 8364"),
            Some("HCI - 26, HCI - 27, HCI - 28"),
        );
        let delta = to_delta(&row, FieldCardinality::Multi).unwrap();
        let Delta::Snapshot {
            from_ids,
            from_displays,
            to_ids,
            to_displays,
        } = delta
        else {
            panic!("expected Snapshot");
        };
        assert_eq!(from_displays, vec!["HCI - 26", "HCI - 27"]);
        assert_eq!(to_displays, vec!["HCI - 26", "HCI - 27", "HCI - 28"]);
        assert_eq!(from_ids, vec!["8066", "8363"]);
        assert_eq!(to_ids, vec!["8066", "8363", "8364"]);
    }

    #[test]
    fn sprint_legacy_first_add_snapshot() {
        // First value added — fromString is NULL/empty, toString is one element (no comma yet).
        // Since NEITHER side contains ", ", it falls through to Add — which is semantically the
        // same as Snapshot([], [X]). Either is acceptable; current behavior: Add.
        let row = item("customfield_10100", None, None, Some("8066"), Some("HCI - 26"));
        let delta = to_delta(&row, FieldCardinality::Multi).unwrap();
        assert!(matches!(delta, Delta::Add { .. }));
    }

    #[test]
    fn roadmap_quarter_list_becomes_snapshot() {
        // "Roadmap candidate Quarter" — another legacy comma-list custom field.
        let row = item(
            "customfield_13237",
            Some("2026 Q1"),
            Some("2026 Q1"),
            Some("2026 Q1, 2026 Q2"),
            Some("2026 Q1, 2026 Q2"),
        );
        let delta = to_delta(&row, FieldCardinality::Multi).unwrap();
        let Delta::Snapshot { to_displays, .. } = delta else {
            panic!("expected Snapshot");
        };
        assert_eq!(to_displays, vec!["2026 Q1", "2026 Q2"]);
    }

    #[test]
    fn snapshot_shrink_detected() {
        let row = item(
            "customfield_10100",
            Some("1, 2, 3"),
            Some("A, B, C"),
            Some("1, 3"),
            Some("A, C"),
        );
        let delta = to_delta(&row, FieldCardinality::Multi).unwrap();
        let Delta::Snapshot { to_displays, .. } = delta else {
            panic!("expected Snapshot");
        };
        assert_eq!(to_displays, vec!["A", "C"]);
    }

    #[test]
    fn snapshot_clear_detected() {
        let row = item("customfield_10100", Some("8066, 8363"), Some("HCI - 26, HCI - 27"), None, None);
        let delta = to_delta(&row, FieldCardinality::Multi).unwrap();
        let Delta::Snapshot {
            from_displays,
            to_displays,
            ..
        } = delta
        else {
            panic!("expected Snapshot");
        };
        assert_eq!(from_displays, vec!["HCI - 26", "HCI - 27"]);
        assert!(to_displays.is_empty());
    }

    #[test]
    fn multi_value_with_degenerate_null_null_returns_none() {
        let row = item("labels", None, None, None, None);
        assert!(to_delta(&row, FieldCardinality::Multi).is_none());
    }
}
