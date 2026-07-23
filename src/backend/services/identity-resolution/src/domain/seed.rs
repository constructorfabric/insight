//! Persons-seed domain: group source-account profiles and resolve each group to
//! a `person_id` — the **write-side** identity resolution (what the read side
//! only looks up). Pure logic, no DB / IO, mirroring the .NET
//! `EmailProfileResolver` + `PersonAssignmentResolver`.

// Built incrementally: the infra/API slices that consume these types land in
// later commits, so allow dead_code until they're wired in.
#![allow(dead_code)]

use std::collections::HashMap;

use sea_orm::prelude::DateTime;
use uuid::Uuid;

/// Identifies one source-native account: the source instance (`source_type` +
/// `source_id`) plus the account's native id within it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceAccountKey {
    pub source_type: String,
    pub source_id: Uuid,
    pub account_id: String,
}

/// One raw observation from `identity.identity_inputs` (what the connectors
/// emit). `synced_at` is monotonic per account; `is_delete` marks a tombstone
/// (signal only — never persisted). Ported from the .NET `IdentityInputRow`.
#[derive(Debug, Clone)]
pub struct IdentityInputRow {
    pub source_type: String,
    pub source_id: Uuid,
    pub source_account_id: String,
    pub value_type: String,
    pub value: String,
    pub synced_at: DateTime,
    pub is_delete: bool,
}

/// One account folded from the raw input stream: its current email, whether it
/// is closed (latest observation is a tombstone), and the upsert observations
/// to persist once the group's `person_id` is resolved.
#[derive(Debug, Clone)]
pub struct SeedProfile {
    pub account: SourceAccountKey,
    pub latest_email: Option<String>,
    pub is_closed: bool,
    pub observations: Vec<IdentityInputRow>,
}

/// A resolved observation ready to append to `persons` — stamped with the
/// assigned `person_id`, routed into one of the three value columns. Consumed by
/// `infra::db::seed_repo::apply`.
#[derive(Debug, Clone)]
pub struct SeedObservationRow {
    pub value_type: String,
    pub source_type: String,
    pub source_id: Uuid,
    pub value_id: Option<String>,
    pub value_full_text: Option<String>,
    pub value: Option<String>,
    pub person_id: Uuid,
    pub author_person_id: Uuid,
    pub reason: Option<String>,
    pub created_at: DateTime,
}

/// Accounts that resolve to the same person — grouped by current email.
#[derive(Debug, Clone)]
pub struct ProfileGroup {
    pub profiles: Vec<SeedProfile>,
}

/// How a group's `person_id` was decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentKind {
    /// An account in the group was already bound to a person (idempotent reuse).
    ReusedKnown,
    /// The group's email already maps to an existing person.
    LinkedByEmail,
    /// A fresh person was minted for the group.
    Minted,
}

/// A group bound to a person, carrying the accounts that share it.
#[derive(Debug, Clone)]
pub struct PersonAssignment {
    pub person_id: Uuid,
    pub kind: AssignmentKind,
    pub profiles: Vec<SeedProfile>,
}

/// Assignments plus per-branch counters (feed the operation summary).
#[derive(Debug, Default)]
pub struct ResolveOutcome {
    pub assignments: Vec<PersonAssignment>,
    pub reused_known: usize,
    pub linked_by_email: usize,
    pub minted: usize,
    pub skipped_closed: usize,
    pub skipped_no_email: usize,
    /// Email groups whose accounts were already bound to *more than one*
    /// existing person; the group is still collapsed onto the first binding
    /// (parity with the .NET resolver), but the conflict is counted + logged
    /// so a silent identity merge is observable.
    pub known_binding_conflicts: usize,
}

/// Case-fold an email for grouping / lookup (ADR-0011: matched
/// case-insensitively). Lowercases only — it does **not** trim, matching the
/// .NET seed path (`StringComparer.OrdinalIgnoreCase` + "store as-is"):
/// surrounding whitespace is significant, so two accounts that differ only by
/// stray whitespace resolve to distinct persons, exactly as the .NET seeder
/// does. Blank/whitespace-only values are treated as "no email" by the callers.
/// The infra layer must key the `email → person` map with the same function.
#[must_use]
pub fn normalize_email(email: &str) -> String {
    email.to_lowercase()
}

/// Group profiles that share the same current email into one group; profiles
/// with no (or blank) email each become a singleton group. Mirrors the .NET
/// `EmailProfileResolver`.
#[must_use]
pub fn group_by_email(profiles: Vec<SeedProfile>) -> Vec<ProfileGroup> {
    let mut by_email: HashMap<String, Vec<SeedProfile>> = HashMap::new();
    let mut singletons: Vec<ProfileGroup> = Vec::new();

    for profile in profiles {
        match profile
            .latest_email
            .as_deref()
            .map(normalize_email)
            .filter(|e| !e.trim().is_empty())
        {
            Some(email) => by_email.entry(email).or_default().push(profile),
            None => singletons.push(ProfileGroup {
                profiles: vec![profile],
            }),
        }
    }

    let mut groups: Vec<ProfileGroup> = by_email
        .into_values()
        .map(|profiles| ProfileGroup { profiles })
        .collect();
    groups.extend(singletons);
    groups
}

/// Resolve each group to a `person_id` via the .NET four-branch classification,
/// in priority order: reuse an already-bound account (idempotent); else link to
/// the person the group's email already maps to; else mint a fresh person when
/// at least one profile is active; else skip (no email, or all closed). `mint`
/// is injected so tests are deterministic.
#[must_use]
pub fn resolve_assignments(
    groups: Vec<ProfileGroup>,
    known: &HashMap<SourceAccountKey, Uuid>,
    email_to_person: &HashMap<String, Uuid>,
    mut mint: impl FnMut() -> Uuid,
) -> ResolveOutcome {
    let mut out = ResolveOutcome::default();

    for group in groups {
        // 1. Known binding wins — reuse the person for the whole group, even
        //    when the group also has no email (idempotent re-seed). If the
        //    group's accounts are bound to *different* persons (e.g. an email
        //    reassigned after a departure), we still collapse onto the first
        //    binding for .NET parity, but surface the conflict — otherwise the
        //    other account's identity is silently merged away.
        let bound: Vec<Uuid> = group
            .profiles
            .iter()
            .filter_map(|p| known.get(&p.account).copied())
            .collect();
        if let Some(&pid) = bound.first() {
            if bound.iter().any(|&other| other != pid) {
                out.known_binding_conflicts += 1;
                tracing::warn!(
                    person_id = %pid,
                    accounts = group.profiles.len(),
                    "persons-seed: group accounts bound to multiple persons; \
                     collapsing onto the first (possible identity merge)"
                );
            }
            out.reused_known += group.profiles.len();
            out.assignments.push(PersonAssignment {
                person_id: pid,
                kind: AssignmentKind::ReusedKnown,
                profiles: group.profiles,
            });
            continue;
        }

        // The group's email — shared by every profile in an email group;
        // singleton no-email groups have none. (`first` is always `Some` here —
        // groups are never empty by construction — but avoid the panicking index.)
        let email = group
            .profiles
            .first()
            .and_then(|p| p.latest_email.as_deref())
            .map(normalize_email)
            .filter(|e| !e.trim().is_empty());
        let Some(email) = email else {
            out.skipped_no_email += group.profiles.len();
            continue;
        };

        // 2. Email matches an existing person → link.
        if let Some(&pid) = email_to_person.get(&email) {
            out.linked_by_email += group.profiles.len();
            out.assignments.push(PersonAssignment {
                person_id: pid,
                kind: AssignmentKind::LinkedByEmail,
                profiles: group.profiles,
            });
            continue;
        }

        // 3/4. No binding, no email match — mint only if at least one profile is
        //      active; a wholly-closed group creates no person.
        if group.profiles.iter().any(|p| !p.is_closed) {
            out.minted += group.profiles.len();
            let person_id = mint();
            out.assignments.push(PersonAssignment {
                person_id,
                kind: AssignmentKind::Minted,
                profiles: group.profiles,
            });
        } else {
            out.skipped_closed += group.profiles.len();
        }
    }

    out
}

/// Reason stamped on observations linked via the email branch (forensics).
pub const AUTO_SEED_LINK_REASON: &str = "auto-seed-link";

/// Route an observation value into exactly one of the three `persons` value
/// columns by `value_type` (ported from the .NET `ValueRouting`): identifier
/// types → `value_id`; human-readable attributes → `value_full_text`; the rest
/// → the uncapped `value` (TEXT). Over-limit values return all-`None` (dropped,
/// never truncated). Returns `(value_id, value_full_text, value)`.
#[must_use]
pub fn route_value(
    value_type: &str,
    value: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    const MAX_VALUE_ID_LEN: usize = 320; // VARCHAR(320)
    const MAX_VALUE_FULL_TEXT_LEN: usize = 512; // VARCHAR(512)
    const VALUE_ID_TYPES: [&str; 7] = [
        "id",
        "email",
        "username",
        "employee_id",
        "parent_email",
        "parent_id",
        "parent_person_id",
    ];
    const VALUE_FULL_TEXT_TYPES: [&str; 7] = [
        "display_name",
        "first_name",
        "last_name",
        "department",
        "division",
        "job_title",
        "status",
    ];

    let len = value.chars().count();
    if VALUE_ID_TYPES.contains(&value_type) {
        if len > MAX_VALUE_ID_LEN {
            return (None, None, None);
        }
        return (Some(value.to_owned()), None, None);
    }
    if VALUE_FULL_TEXT_TYPES.contains(&value_type) {
        if len > MAX_VALUE_FULL_TEXT_LEN {
            return (None, None, None);
        }
        return (None, Some(value.to_owned()), None);
    }
    (None, None, Some(value.to_owned()))
}

/// Fold the raw input stream (delivered **latest-first per account**) into one
/// [`SeedProfile`] per source account: the first row seen marks the account
/// closed (tombstone latest), the first email row's value is the current email,
/// and tombstone rows are signal-only (never persisted). Mirrors the .NET
/// `AccountAccumulator`.
#[must_use]
pub fn build_profiles(rows: Vec<IdentityInputRow>) -> Vec<SeedProfile> {
    struct Acc {
        latest_email: Option<String>,
        is_closed: bool,
        saw_any: bool,
        upserts: Vec<IdentityInputRow>,
    }

    let mut by_account: HashMap<SourceAccountKey, Acc> = HashMap::new();
    for row in rows {
        let key = SourceAccountKey {
            source_type: row.source_type.clone(),
            source_id: row.source_id,
            account_id: row.source_account_id.clone(),
        };
        let acc = by_account.entry(key).or_insert_with(|| Acc {
            latest_email: None,
            is_closed: false,
            saw_any: false,
            upserts: Vec::new(),
        });
        if !acc.saw_any {
            acc.is_closed = row.is_delete; // first row = latest observation
            acc.saw_any = true;
        }
        if row.value_type == "email" && acc.latest_email.is_none() && !row.value.trim().is_empty() {
            acc.latest_email = Some(row.value.clone()); // stored as-is (ADR-0011)
        }
        if !row.is_delete {
            acc.upserts.push(row);
        }
    }

    by_account
        .into_iter()
        .map(|(account, acc)| SeedProfile {
            account,
            latest_email: acc.latest_email,
            is_closed: acc.is_closed,
            observations: acc.upserts,
        })
        .collect()
}

/// Turn resolved assignments into the observation rows to append to `persons`:
/// each upsert observation, routed into its value column and stamped with the
/// group's `person_id` and the seed author. Email-linked assignments carry the
/// `auto-seed-link` reason; reused / minted carry an empty reason (matching the
/// .NET seeder). Over-limit values are dropped. Mirrors `BuildObservationRows`.
#[must_use]
pub fn assignments_to_rows(
    assignments: &[PersonAssignment],
    author_person_id: Uuid,
) -> Vec<SeedObservationRow> {
    let mut rows = Vec::new();
    for assignment in assignments {
        let reason = if assignment.kind == AssignmentKind::LinkedByEmail {
            AUTO_SEED_LINK_REASON
        } else {
            ""
        };
        for profile in &assignment.profiles {
            for obs in &profile.observations {
                let (value_id, value_full_text, value) = route_value(&obs.value_type, &obs.value);
                if value_id.is_none() && value_full_text.is_none() && value.is_none() {
                    continue; // oversized — dropped per the routing rule
                }
                rows.push(SeedObservationRow {
                    value_type: obs.value_type.clone(),
                    source_type: obs.source_type.clone(),
                    source_id: obs.source_id,
                    value_id,
                    value_full_text,
                    value,
                    person_id: assignment.person_id,
                    author_person_id,
                    reason: Some(reason.to_owned()),
                    created_at: obs.synced_at,
                });
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prof(source_type: &str, account_id: &str, email: Option<&str>, closed: bool) -> SeedProfile {
        SeedProfile {
            account: SourceAccountKey {
                source_type: source_type.to_owned(),
                source_id: Uuid::from_u128(1),
                account_id: account_id.to_owned(),
            },
            latest_email: email.map(str::to_owned),
            is_closed: closed,
            observations: Vec::new(),
        }
    }

    /// A minting factory yielding Uuid(1), Uuid(2), … deterministically.
    fn counter() -> impl FnMut() -> Uuid {
        let mut n = 0u128;
        move || {
            n += 1;
            Uuid::from_u128(n)
        }
    }

    #[test]
    fn groups_by_email_case_insensitively_singletons_for_no_email() {
        let groups = group_by_email(vec![
            prof("bamboohr", "1", Some("anna@corp.com"), false),
            prof("slack", "U1", Some("ANNA@corp.com"), false), // same person, diff case
            prof("bamboohr", "2", Some("boris@corp.com"), false),
            prof("zoom", "Z1", None, false), // no email → singleton
        ]);
        assert_eq!(
            groups.len(),
            3,
            "anna(x2 merged) + boris + no-email singleton"
        );
        let anna = groups
            .iter()
            .find(|g| g.profiles.iter().any(|p| p.account.account_id == "U1"));
        assert!(
            anna.is_some_and(|g| g.profiles.len() == 2),
            "case variants merge into one group"
        );
    }

    #[test]
    fn emails_are_case_folded_but_not_trimmed() {
        // Case variants merge; a trailing-space variant stays a separate group —
        // parity with the .NET seeder (OrdinalIgnoreCase + store-as-is, no trim).
        let groups = group_by_email(vec![
            prof("bamboohr", "1", Some("anna@corp.com"), false),
            prof("slack", "U1", Some("ANNA@corp.com"), false), // case → merges
            prof("zoom", "Z1", Some("anna@corp.com "), false), // trailing space → distinct
        ]);
        assert_eq!(
            groups.len(),
            2,
            "case merges into one group; trailing-space stays separate"
        );
    }

    #[test]
    fn mints_new_person_for_active_unknown_group() {
        let groups = group_by_email(vec![prof("bamboohr", "1", Some("anna@corp.com"), false)]);
        let out = resolve_assignments(groups, &HashMap::new(), &HashMap::new(), counter());
        assert_eq!(out.minted, 1);
        assert_eq!(out.assignments.len(), 1);
        assert_eq!(out.assignments[0].kind, AssignmentKind::Minted);
        assert_eq!(out.assignments[0].person_id, Uuid::from_u128(1));
    }

    #[test]
    fn skips_wholly_closed_and_no_email_groups() {
        let closed = group_by_email(vec![prof("bamboohr", "1", Some("gone@corp.com"), true)]);
        let out = resolve_assignments(closed, &HashMap::new(), &HashMap::new(), counter());
        assert_eq!(out.skipped_closed, 1);
        assert!(out.assignments.is_empty(), "closed accounts never mint");

        let no_email = group_by_email(vec![prof("zoom", "Z1", None, false)]);
        let out2 = resolve_assignments(no_email, &HashMap::new(), &HashMap::new(), counter());
        assert_eq!(out2.skipped_no_email, 1);
        assert!(out2.assignments.is_empty());
    }

    #[test]
    fn reuses_known_account_binding_over_email() {
        let p = prof("bamboohr", "1", Some("anna@corp.com"), false);
        let mut known = HashMap::new();
        known.insert(p.account.clone(), Uuid::from_u128(42)); // already bound
        let mut email_map = HashMap::new();
        email_map.insert("anna@corp.com".to_owned(), Uuid::from_u128(99)); // different person!

        let out = resolve_assignments(group_by_email(vec![p]), &known, &email_map, counter());
        assert_eq!(out.reused_known, 1);
        assert_eq!(out.linked_by_email, 0);
        // Known binding wins over the email map.
        assert_eq!(out.assignments[0].person_id, Uuid::from_u128(42));
        assert_eq!(out.assignments[0].kind, AssignmentKind::ReusedKnown);
    }

    #[test]
    fn links_new_account_to_existing_person_by_email() {
        // A brand-new account (not in `known`) whose email is already known.
        let groups = group_by_email(vec![prof("github", "gh1", Some("Anna@corp.com"), false)]);
        let mut email_map = HashMap::new();
        email_map.insert("anna@corp.com".to_owned(), Uuid::from_u128(7)); // normalized key
        let out = resolve_assignments(groups, &HashMap::new(), &email_map, counter());
        assert_eq!(out.linked_by_email, 1);
        assert_eq!(out.assignments[0].kind, AssignmentKind::LinkedByEmail);
        assert_eq!(out.assignments[0].person_id, Uuid::from_u128(7));
    }

    #[test]
    fn whole_email_group_binds_to_one_person_via_known_member() {
        // Two accounts share an email; only one is already known → the whole
        // group reuses that person (the new account joins the same person).
        let known_acc = prof("slack", "U1", Some("anna@corp.com"), false);
        let new_acc = prof("github", "gh1", Some("anna@corp.com"), false);
        let mut known = HashMap::new();
        known.insert(known_acc.account.clone(), Uuid::from_u128(5));

        let out = resolve_assignments(
            group_by_email(vec![known_acc, new_acc]),
            &known,
            &HashMap::new(),
            counter(),
        );
        assert_eq!(out.assignments.len(), 1);
        assert_eq!(out.reused_known, 2, "both accounts in the group counted");
        assert_eq!(out.assignments[0].person_id, Uuid::from_u128(5));
        assert_eq!(out.assignments[0].profiles.len(), 2);
        assert_eq!(
            out.known_binding_conflicts, 0,
            "single binding, no conflict"
        );
    }

    #[test]
    fn multi_person_binding_conflict_collapses_and_is_counted() {
        // Two accounts share an email but are already bound to two *different*
        // persons (email reassigned after a departure). The group still
        // collapses onto the first binding (.NET parity), but the conflict is
        // counted so the silent identity merge is observable.
        let acc_a = prof("slack", "U1", Some("anna@corp.com"), false);
        let acc_b = prof("github", "gh1", Some("anna@corp.com"), false);
        let mut known = HashMap::new();
        known.insert(acc_a.account.clone(), Uuid::from_u128(5));
        known.insert(acc_b.account.clone(), Uuid::from_u128(6));

        let out = resolve_assignments(
            group_by_email(vec![acc_a, acc_b]),
            &known,
            &HashMap::new(),
            counter(),
        );
        assert_eq!(out.assignments.len(), 1, "group collapsed onto one person");
        assert_eq!(out.reused_known, 2);
        assert_eq!(
            out.known_binding_conflicts, 1,
            "conflict detected + counted"
        );
    }

    fn input(
        source_type: &str,
        account_id: &str,
        value_type: &str,
        value: &str,
        is_delete: bool,
        synced_at: DateTime,
    ) -> IdentityInputRow {
        IdentityInputRow {
            source_type: source_type.to_owned(),
            source_id: Uuid::from_u128(1),
            source_account_id: account_id.to_owned(),
            value_type: value_type.to_owned(),
            value: value.to_owned(),
            synced_at,
            is_delete,
        }
    }

    #[test]
    fn route_value_by_type_and_drops_oversized() {
        assert_eq!(
            route_value("email", "a@b.com"),
            (Some("a@b.com".to_owned()), None, None)
        );
        assert_eq!(
            route_value("display_name", "Ann Smith"),
            (None, Some("Ann Smith".to_owned()), None)
        );
        assert_eq!(
            route_value("custom", "whatever"),
            (None, None, Some("whatever".to_owned()))
        );
        let huge = "x".repeat(321);
        assert_eq!(
            route_value("email", &huge),
            (None, None, None),
            "over 320 chars → dropped, not truncated"
        );
    }

    #[test]
    fn build_profiles_folds_latest_first() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let profiles = build_profiles(vec![
            input("bamboohr", "5001", "email", "new@corp.com", false, t), // latest
            input("bamboohr", "5001", "email", "old@corp.com", false, t), // older, ignored
            input("bamboohr", "5001", "status", "Active", false, t),
            input("bamboohr", "5001", "username", "tomb", true, t), // tombstone
        ]);
        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        assert_eq!(p.latest_email.as_deref(), Some("new@corp.com"));
        assert!(!p.is_closed);
        assert_eq!(p.observations.len(), 3, "tombstone is not persisted");
        Ok(())
    }

    #[test]
    fn build_profiles_marks_closed_when_latest_is_tombstone() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let profiles = build_profiles(vec![input("slack", "U1", "email", "x@y.com", true, t)]);
        assert!(profiles[0].is_closed, "latest observation is a tombstone");
        assert!(
            profiles[0].observations.is_empty(),
            "tombstone not persisted"
        );
        // Email is still captured even from a tombstone row (matches .NET).
        assert_eq!(profiles[0].latest_email.as_deref(), Some("x@y.com"));
        Ok(())
    }

    #[test]
    fn assignments_to_rows_stamps_person_routes_and_reason() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let profile = SeedProfile {
            account: SourceAccountKey {
                source_type: "bamboohr".to_owned(),
                source_id: Uuid::from_u128(1),
                account_id: "5001".to_owned(),
            },
            latest_email: Some("a@b.com".to_owned()),
            is_closed: false,
            observations: vec![
                input("bamboohr", "5001", "email", "a@b.com", false, t),
                input("bamboohr", "5001", "display_name", "Ann Smith", false, t),
                input("bamboohr", "5001", "email", &"x".repeat(321), false, t), // oversized
            ],
        };
        let minted = PersonAssignment {
            person_id: Uuid::from_u128(10),
            kind: AssignmentKind::Minted,
            profiles: vec![profile.clone()],
        };
        let linked = PersonAssignment {
            person_id: Uuid::from_u128(20),
            kind: AssignmentKind::LinkedByEmail,
            profiles: vec![profile],
        };

        let rows = assignments_to_rows(&[minted, linked], Uuid::from_u128(99));
        // 2 valid obs (email + display_name; oversized dropped) × 2 assignments.
        assert_eq!(rows.len(), 4);
        // Routing: email → value_id, display_name → value_full_text.
        assert!(rows.iter().any(|r| r.value_type == "email"
            && r.value_id.as_deref() == Some("a@b.com")
            && r.value_full_text.is_none()));
        assert!(
            rows.iter().any(|r| r.value_type == "display_name"
                && r.value_full_text.as_deref() == Some("Ann Smith"))
        );
        // Minted rows: empty reason, seed author.
        assert!(
            rows.iter()
                .filter(|r| r.person_id == Uuid::from_u128(10))
                .all(|r| r.reason.as_deref() == Some("")
                    && r.author_person_id == Uuid::from_u128(99))
        );
        // Email-linked rows: auto-seed-link reason.
        assert!(
            rows.iter()
                .filter(|r| r.person_id == Uuid::from_u128(20))
                .all(|r| r.reason.as_deref() == Some("auto-seed-link"))
        );
        Ok(())
    }

    #[test]
    fn pure_pipeline_build_group_resolve_rows() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        // Anna across two sources sharing an email; empty persons → mint once.
        let profiles = build_profiles(vec![
            input("bamboohr", "5001", "email", "anna@corp.com", false, t),
            input("bamboohr", "5001", "display_name", "Anna P", false, t),
            input("slack", "U777", "email", "anna@corp.com", false, t),
        ]);
        let out = resolve_assignments(
            group_by_email(profiles),
            &HashMap::new(),
            &HashMap::new(),
            counter(),
        );
        assert_eq!(out.assignments.len(), 1, "one person for the email group");
        assert_eq!(out.minted, 2, "both accounts counted");

        let person = out.assignments[0].person_id;
        let obs_rows = assignments_to_rows(&out.assignments, Uuid::from_u128(99));
        assert!(!obs_rows.is_empty());
        assert!(
            obs_rows.iter().all(|r| r.person_id == person),
            "every observation stamped with the one resolved person"
        );
        Ok(())
    }
}
