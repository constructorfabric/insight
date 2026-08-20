//! Persons-seed domain: group source-account profiles and resolve each group to
//! a `person_id` — the **write-side** identity resolution (what the read side
//! only looks up). Pure logic, no DB / IO. Ported from the .NET
//! `EmailProfileResolver` + `PersonAssignmentResolver`, with one deliberate
//! deviation: divergent e-mail groups keep per-account bindings (classified by
//! binding author) instead of collapsing onto the first binding.

use std::collections::HashMap;

use sea_orm::prelude::DateTime;
use uuid::Uuid;

use super::observation_slot::SlotAllocator;
use super::provenance::{Provenance, ROSTER_MINT_REASON};
use super::resolution::{BINDING_VALUE_TYPE, EXCLUDED_PERSON};
use super::roster::RosterSource;

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

/// An account's current binding as loaded from `persons`: the person, who
/// authored the binding row, and how it came about. Authorship decides conflict
/// classification — an operator-authored binding marks divergence inside an
/// e-mail group as an intentional, settled state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownBinding {
    pub person_id: Uuid,
    pub author_person_id: Uuid,
    pub provenance: Provenance,
}

impl KnownBinding {
    /// The all-zero author is the seed sentinel; any real UUID is an operator.
    #[must_use]
    pub fn is_operator_authored(&self) -> bool {
        !self.author_person_id.is_nil()
    }

    /// A binding automation wrote and nobody has confirmed. It is what the
    /// review queue asks about, the only kind an address may override, and the
    /// only kind whose reason survives a re-emission.
    #[must_use]
    pub fn is_unconfirmed_mint(&self) -> bool {
        !self.is_operator_authored() && self.provenance != Provenance::Resolved
    }
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
    /// A fresh person was minted for a roster account carrying no address —
    /// unverifiable by automation, so the operator is asked to confirm it.
    MintedFromRoster,
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
    /// Email groups whose accounts are bound to *more than one* person with no
    /// operator-authored binding explaining it. Each account keeps its own
    /// binding (never collapsed); the group is counted + logged for review.
    pub known_binding_conflicts: usize,
    /// Divergent e-mail groups where at least one binding is operator-authored
    /// — an intentional split, kept silent (not a conflict).
    pub operator_settled_groups: usize,
    /// Unbound accounts inside a divergent e-mail group: their e-mail is
    /// contested evidence, so they are not auto-linked to anyone.
    pub skipped_contested_email: usize,
    /// Accounts bound to the excluded person (ADR-0003). They are not persons:
    /// nothing is re-emitted for them, and their values link nobody —
    /// automation may not spread an operator's exclusion to new accounts.
    pub skipped_excluded: usize,
    /// Roster accounts with no address that were minted a person anyway. Each
    /// one reaches the review queue: the roster says a human exists, nothing
    /// says they are not already on it under another account.
    pub minted_from_roster: usize,
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

/// Resolve each group to a `person_id`, in priority order: reuse already-bound
/// accounts (idempotent — each account keeps **its own** binding, never
/// collapsed across a divergent group); else link to the person the group's
/// email already maps to; else mint a fresh person when at least one profile
/// is active; else skip (all closed, or no address and no roster to vouch for
/// the account). `mint` is injected so tests are deterministic.
///
/// `roster` names the source allowed to mint without an address. `None` — the
/// default — keeps an addressless account unresolved, as before.
#[must_use]
pub fn resolve_assignments(
    groups: Vec<ProfileGroup>,
    known: &HashMap<SourceAccountKey, KnownBinding>,
    email_to_person: &HashMap<String, Uuid>,
    roster: Option<&RosterSource>,
    mut mint: impl FnMut() -> Uuid,
) -> ResolveOutcome {
    let mut out = ResolveOutcome::default();

    for group in groups {
        // 0. Excluded accounts (bound to the sentinel) are not persons: they
        //    contribute no new observations, and they leave the group before
        //    any linking decision so their values claim nobody. The exclusion
        //    itself stays in force — its journal row is already the latest.
        let (excluded, remaining): (Vec<_>, Vec<_>) = group.profiles.into_iter().partition(|p| {
            known
                .get(&p.account)
                .is_some_and(|b| b.person_id == EXCLUDED_PERSON)
        });
        out.skipped_excluded += excluded.len();
        if remaining.is_empty() {
            continue;
        }

        // The group's address — shared by every profile in an e-mail group; an
        // addressless group is a singleton with none. (`first` is always `Some`
        // here, groups being non-empty by construction, but avoid the panicking
        // index.)
        let email = remaining
            .first()
            .and_then(|p| p.latest_email.as_deref())
            .map(normalize_email)
            .filter(|e| !e.trim().is_empty());

        // The person that address already names, if any. A binding automation
        // wrote and nobody confirmed gives way to it: leaving such an account on
        // its minted person would hand one human two persons AND give the
        // address two claimants, which resolves to NOBODY downstream — so the
        // human's activity would reach no metric at all.
        let claimed_by = email
            .as_deref()
            .and_then(|e| email_to_person.get(e).copied())
            .filter(|pid| *pid != EXCLUDED_PERSON);

        // 1. Known bindings win — and each bound account keeps its own person.
        //    A group whose accounts are bound to different persons is an
        //    intentional split when any binding is operator-authored (ADR-0003);
        //    otherwise it is a conflict to surface. Either way the e-mail is
        //    contested evidence, so unbound group members are not auto-linked.
        let (bound, unbound): (Vec<_>, Vec<_>) = remaining.into_iter().partition(|p| {
            known
                .get(&p.account)
                .is_some_and(|b| !yields_to_address(b, claimed_by))
        });
        if !bound.is_empty() {
            let bindings: Vec<KnownBinding> = bound.iter().map(|p| known[&p.account]).collect();
            let first_person = bindings[0].person_id;
            let divergent = bindings.iter().any(|b| b.person_id != first_person);

            if divergent {
                record_divergent_group(bound, &unbound, &bindings, known, &mut out);
                continue;
            }

            let mut profiles = bound;
            profiles.extend(unbound);
            out.reused_known += profiles.len();
            out.assignments.push(PersonAssignment {
                person_id: first_person,
                kind: AssignmentKind::ReusedKnown,
                profiles,
            });
            continue;
        }
        let group = ProfileGroup { profiles: unbound };

        if email.is_none() {
            record_addressless_group(group, roster, &mut mint, &mut out);
            continue;
        }

        // 2. Email matches an existing person → link. A map entry naming the
        //    excluded sentinel (legacy rows from before exclusions stopped
        //    re-emitting) is no person and links nobody — fall through to mint.
        if let Some(pid) = claimed_by {
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

/// The reason to stamp on one account's observations.
///
/// INVARIANT: an unconfirmed mint keeps saying so for as long as it stands. The
/// binding read takes the LATEST `id` row, and a source re-emits that row on
/// every change it makes to the account — so stamping the assignment's own
/// reason would retire the operator's review item, and un-flag the person the
/// merge picker greys out, with no decision behind either.
fn reason_for(
    assignment: &PersonAssignment,
    profile: &SeedProfile,
    known: &HashMap<SourceAccountKey, KnownBinding>,
) -> &'static str {
    let carried = known
        .get(&profile.account)
        .filter(|binding| binding.person_id == assignment.person_id)
        .filter(|binding| binding.is_unconfirmed_mint())
        .and_then(|binding| binding.provenance.reason_code());
    if let Some(reason) = carried {
        return reason;
    }

    match assignment.kind {
        AssignmentKind::LinkedByEmail => AUTO_SEED_LINK_REASON,
        AssignmentKind::MintedFromRoster => ROSTER_MINT_REASON,
        AssignmentKind::ReusedKnown | AssignmentKind::Minted => "",
    }
}

/// Record a group whose bound accounts name different persons. Each keeps its
/// own binding — automation never collapses a split — and the group's address is
/// contested evidence, so its unbound members are linked to nobody. An
/// operator-authored binding among them makes the split an intentional one
/// (ADR-0003), which is counted but not surfaced.
fn record_divergent_group(
    bound: Vec<SeedProfile>,
    unbound: &[SeedProfile],
    bindings: &[KnownBinding],
    known: &HashMap<SourceAccountKey, KnownBinding>,
    out: &mut ResolveOutcome,
) {
    if bindings.iter().any(KnownBinding::is_operator_authored) {
        out.operator_settled_groups += 1;
    } else {
        out.known_binding_conflicts += 1;
        tracing::warn!(
            accounts = bound.len(),
            "persons-seed: group accounts bound to multiple persons with no \
             operator decision; keeping each binding, surfacing for review"
        );
    }

    let mut by_person: HashMap<Uuid, Vec<SeedProfile>> = HashMap::new();
    for profile in bound {
        let person = known[&profile.account].person_id;
        by_person.entry(person).or_default().push(profile);
    }
    for (person_id, profiles) in by_person {
        out.reused_known += profiles.len();
        out.assignments.push(PersonAssignment {
            person_id,
            kind: AssignmentKind::ReusedKnown,
            profiles,
        });
    }

    if !unbound.is_empty() {
        out.skipped_contested_email += unbound.len();
        tracing::warn!(
            accounts = unbound.len(),
            "persons-seed: e-mail contested between persons; not auto-linking \
             new accounts"
        );
    }
}

/// Whether an existing binding must give way to the person an address already
/// names. Only a binding nobody has confirmed does, and only in favour of a
/// DIFFERENT person: an operator's decision is final, and a binding the address
/// itself produced is already the answer.
fn yields_to_address(binding: &KnownBinding, claimed_by: Option<Uuid>) -> bool {
    let Some(person) = claimed_by else {
        return false;
    };

    person != binding.person_id && binding.is_unconfirmed_mint()
}

/// Resolve a group carrying no address. Only the roster may mint for one: an
/// address is the sole key automation can match on, so every other source's
/// addressless account is left for an operator.
///
/// The account must also state its own id. That observation becomes the binding
/// row, and minting without one leaves a person no account points at —
/// invisible to every later run, and to the operator who would have to repair
/// it.
fn record_addressless_group(
    group: ProfileGroup,
    roster: Option<&RosterSource>,
    mint: &mut impl FnMut() -> Uuid,
    out: &mut ResolveOutcome,
) {
    let vouched_for = roster.is_some_and(|roster| {
        group
            .profiles
            .iter()
            .all(|p| roster.speaks_for(&p.account.source_type) && states_a_bindable_id(p))
    });
    if !vouched_for {
        out.skipped_no_email += group.profiles.len();
        return;
    }

    // Deactivated at its source: the roster lists no human to add.
    if !group.profiles.iter().any(|p| !p.is_closed) {
        out.skipped_closed += group.profiles.len();
        return;
    }

    out.minted_from_roster += group.profiles.len();
    out.assignments.push(PersonAssignment {
        person_id: mint(),
        kind: AssignmentKind::MintedFromRoster,
        profiles: group.profiles,
    });
}

/// Whether the source states an id for this account that will actually land in
/// `persons` as its binding row.
///
/// Presence is not enough: `route_value` drops an over-long id rather than
/// truncating it, and a mint whose binding row was dropped leaves a person no
/// account points at — with no address to recover it by, every later run mints
/// another one.
fn states_a_bindable_id(profile: &SeedProfile) -> bool {
    profile.observations.iter().any(|o| {
        o.value_type == BINDING_VALUE_TYPE && route_value(BINDING_VALUE_TYPE, &o.value).0.is_some()
    })
}

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
/// .NET seeder). Over-limit values are dropped.
///
/// The natural observation key ends in `created_at` and carries no account
/// discriminator, so two accounts of one source resolving to one person at the
/// same `synced_at` would collide on their `value_type='id'` rows and
/// `INSERT IGNORE` would drop one binding. Rows are therefore nudged forward by
/// whole microseconds until the key is unique within the batch — the smallest
/// step `DATETIME(6)` can store, keeping observation chronology intact.
#[must_use]
pub fn assignments_to_rows(
    assignments: &[PersonAssignment],
    author_person_id: Uuid,
    known: &HashMap<SourceAccountKey, KnownBinding>,
) -> Vec<SeedObservationRow> {
    let mut rows = Vec::new();
    let mut slots = SlotAllocator::new();

    for assignment in assignments {
        for profile in &assignment.profiles {
            let reason = reason_for(assignment, profile, known);
            for obs in &profile.observations {
                let (value_id, value_full_text, value) = route_value(&obs.value_type, &obs.value);
                if value_id.is_none() && value_full_text.is_none() && value.is_none() {
                    continue; // oversized — dropped per the routing rule
                }

                let created_at = slots.claim(
                    assignment.person_id,
                    &obs.source_type,
                    obs.source_id,
                    &obs.value_type,
                    obs.synced_at,
                );

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
                    created_at,
                });
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    /// Resolve with no roster configured — the default, and what every case
    /// about address matching wants: an addressless account stays unresolved.
    fn resolve_without_roster(
        groups: Vec<ProfileGroup>,
        known: &HashMap<SourceAccountKey, KnownBinding>,
        email_to_person: &HashMap<String, Uuid>,
        mint: impl FnMut() -> Uuid,
    ) -> ResolveOutcome {
        resolve_assignments(groups, known, email_to_person, None, mint)
    }

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
    fn seed_bound(person: u128) -> KnownBinding {
        KnownBinding {
            person_id: Uuid::from_u128(person),
            author_person_id: Uuid::nil(),
            provenance: Provenance::Resolved,
        }
    }

    fn operator_bound(person: u128) -> KnownBinding {
        KnownBinding {
            person_id: Uuid::from_u128(person),
            author_person_id: Uuid::from_u128(0xAD_1119),
            provenance: Provenance::Resolved,
        }
    }

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
        let out = resolve_without_roster(groups, &HashMap::new(), &HashMap::new(), counter());
        assert_eq!(out.minted, 1);
        assert_eq!(out.assignments.len(), 1);
        assert_eq!(out.assignments[0].kind, AssignmentKind::Minted);
        assert_eq!(out.assignments[0].person_id, Uuid::from_u128(1));
    }

    #[test]
    fn skips_wholly_closed_and_no_email_groups() {
        let closed = group_by_email(vec![prof("bamboohr", "1", Some("gone@corp.com"), true)]);
        let out = resolve_without_roster(closed, &HashMap::new(), &HashMap::new(), counter());
        assert_eq!(out.skipped_closed, 1);
        assert!(out.assignments.is_empty(), "closed accounts never mint");

        let no_email = group_by_email(vec![prof("zoom", "Z1", None, false)]);
        let out2 = resolve_without_roster(no_email, &HashMap::new(), &HashMap::new(), counter());
        assert_eq!(out2.skipped_no_email, 1);
        assert!(out2.assignments.is_empty());
    }

    #[test]
    fn reuses_known_account_binding_over_email() {
        let p = prof("bamboohr", "1", Some("anna@corp.com"), false);
        let mut known = HashMap::new();
        known.insert(p.account.clone(), seed_bound(42)); // already bound
        let mut email_map = HashMap::new();
        email_map.insert("anna@corp.com".to_owned(), Uuid::from_u128(99)); // different person!

        let out = resolve_without_roster(group_by_email(vec![p]), &known, &email_map, counter());
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
        let out = resolve_without_roster(groups, &HashMap::new(), &email_map, counter());
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
        known.insert(known_acc.account.clone(), seed_bound(5));

        let out = resolve_without_roster(
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
    fn divergent_group_keeps_each_accounts_own_binding() {
        // Two accounts share an email but are bound to two *different* persons
        // with no operator decision: each keeps its own binding (never
        // collapsed) and the group is counted as a conflict for review.
        let acc_a = prof("slack", "U1", Some("anna@corp.com"), false);
        let acc_b = prof("github", "gh1", Some("anna@corp.com"), false);
        let mut known = HashMap::new();
        known.insert(acc_a.account.clone(), seed_bound(5));
        known.insert(acc_b.account.clone(), seed_bound(6));

        let out = resolve_without_roster(
            group_by_email(vec![acc_a, acc_b]),
            &known,
            &HashMap::new(),
            counter(),
        );

        assert_eq!(out.assignments.len(), 2, "one assignment per binding");
        let mut persons: Vec<Uuid> = out.assignments.iter().map(|a| a.person_id).collect();
        persons.sort();
        assert_eq!(persons, vec![Uuid::from_u128(5), Uuid::from_u128(6)]);
        assert_eq!(out.reused_known, 2);
        assert_eq!(
            out.known_binding_conflicts, 1,
            "all-seed divergence surfaces"
        );
        assert_eq!(out.operator_settled_groups, 0);
    }

    #[test]
    fn operator_authored_divergence_is_settled_not_a_conflict() {
        // Same divergence, but one binding was written by an operator (a detach
        // decision): the split is intentional — no conflict is counted.
        let acc_a = prof("slack", "U1", Some("team@corp.com"), false);
        let acc_b = prof("github", "gh1", Some("team@corp.com"), false);
        let mut known = HashMap::new();
        known.insert(acc_a.account.clone(), seed_bound(5));
        known.insert(acc_b.account.clone(), operator_bound(6));

        let out = resolve_without_roster(
            group_by_email(vec![acc_a, acc_b]),
            &known,
            &HashMap::new(),
            counter(),
        );

        assert_eq!(out.assignments.len(), 2);
        assert_eq!(
            out.known_binding_conflicts, 0,
            "operator decision settles it"
        );
        assert_eq!(out.operator_settled_groups, 1);
    }

    #[test]
    fn contested_email_does_not_auto_link_new_accounts() {
        // A divergent group's e-mail is contested evidence: a brand-new account
        // arriving with it is neither linked to either person nor minted — it
        // is left for the operator (skipped + counted).
        let acc_a = prof("slack", "U1", Some("team@corp.com"), false);
        let acc_b = prof("github", "gh1", Some("team@corp.com"), false);
        let newcomer = prof("zoom", "Z9", Some("team@corp.com"), false);
        let mut known = HashMap::new();
        known.insert(acc_a.account.clone(), seed_bound(5));
        known.insert(acc_b.account.clone(), operator_bound(6));

        let out = resolve_without_roster(
            group_by_email(vec![acc_a, acc_b, newcomer]),
            &known,
            &HashMap::new(),
            counter(),
        );

        assert_eq!(out.skipped_contested_email, 1, "newcomer not auto-linked");
        assert_eq!(out.minted, 0);
        assert_eq!(out.reused_known, 2, "bound accounts keep their persons");
        let assigned: usize = out.assignments.iter().map(|a| a.profiles.len()).sum();
        assert_eq!(assigned, 2, "the newcomer is in no assignment");
    }

    fn excluded_bound() -> KnownBinding {
        KnownBinding {
            person_id: EXCLUDED_PERSON,
            author_person_id: Uuid::from_u128(0xAD_1119),
            provenance: Provenance::Resolved,
        }
    }

    #[test]
    fn excluded_accounts_are_skipped_and_their_email_links_nobody() {
        // A bot was excluded by an operator. The seed must not re-emit its
        // observations under the sentinel, and a new account sharing the bot's
        // e-mail must not inherit the exclusion — automation may not decide
        // "not a person". The newcomer is its own fresh person.
        let bot = prof("github", "gh-bot", Some("ci@corp.com"), false);
        let newcomer = prof("jira", "jr-1", Some("ci@corp.com"), false);
        let mut known = HashMap::new();
        known.insert(bot.account.clone(), excluded_bound());

        let out = resolve_without_roster(
            group_by_email(vec![bot, newcomer]),
            &known,
            &HashMap::new(),
            counter(),
        );

        assert_eq!(out.skipped_excluded, 1, "the bot contributes nothing");
        assert_eq!(out.minted, 1, "the newcomer is a fresh person");
        assert_eq!(out.assignments.len(), 1);
        assert_ne!(out.assignments[0].person_id, EXCLUDED_PERSON);
        assert_eq!(out.assignments[0].profiles[0].account.account_id, "jr-1");
    }

    #[test]
    fn an_exclusion_does_not_settle_someone_elses_divergence() {
        // Two automation bindings disagree AND a third account of the group is
        // excluded. The exclusion is an operator decision about the BOT, not
        // about the 5/6 split — the conflict must still surface (the review
        // queue classifies it the same way).
        let acc_a = prof("slack", "U1", Some("team@corp.com"), false);
        let acc_b = prof("github", "gh1", Some("team@corp.com"), false);
        let bot = prof("zoom", "Z9", Some("team@corp.com"), false);
        let mut known = HashMap::new();
        known.insert(acc_a.account.clone(), seed_bound(5));
        known.insert(acc_b.account.clone(), seed_bound(6));
        known.insert(bot.account.clone(), excluded_bound());

        let out = resolve_without_roster(
            group_by_email(vec![acc_a, acc_b, bot]),
            &known,
            &HashMap::new(),
            counter(),
        );

        assert_eq!(out.known_binding_conflicts, 1, "the split still surfaces");
        assert_eq!(out.operator_settled_groups, 0);
        assert_eq!(out.skipped_excluded, 1);
    }

    #[test]
    fn a_legacy_email_map_entry_naming_the_sentinel_links_nobody() {
        // Seeds that ran before exclusions stopped re-emitting may have left
        // e-mail rows under the sentinel; such a map entry is not a person.
        let newcomer = prof("jira", "jr-1", Some("ci@corp.com"), false);
        let mut email_map = HashMap::new();
        email_map.insert("ci@corp.com".to_owned(), EXCLUDED_PERSON);

        let out = resolve_without_roster(
            group_by_email(vec![newcomer]),
            &HashMap::new(),
            &email_map,
            counter(),
        );

        assert_eq!(out.linked_by_email, 0, "the sentinel links nobody");
        assert_eq!(out.minted, 1);
        assert_ne!(out.assignments[0].person_id, EXCLUDED_PERSON);
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
    fn same_source_accounts_on_one_person_get_distinct_timestamps() -> anyhow::Result<()> {
        // Two accounts of one source resolve to one person and were synced at
        // the same instant: their `id` observations share every natural-key
        // column, so without disambiguation INSERT IGNORE would drop one
        // binding. Rows must land on distinct microseconds.
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let mut a = prof("bamboohr", "1", Some("anna@corp.com"), false);
        let mut b = prof("bamboohr", "2", Some("anna@corp.com"), false);
        a.observations = vec![input("bamboohr", "1", "id", "1", false, t)];
        b.observations = vec![input("bamboohr", "2", "id", "2", false, t)];

        let rows = assignments_to_rows(
            &[PersonAssignment {
                person_id: Uuid::from_u128(7),
                kind: AssignmentKind::Minted,
                profiles: vec![a, b],
            }],
            Uuid::nil(),
            &HashMap::new(),
        );

        assert_eq!(rows.len(), 2);
        assert_ne!(
            rows[0].created_at, rows[1].created_at,
            "colliding natural keys must be nudged apart"
        );
        assert_eq!(rows[0].created_at, t, "the first row keeps its own instant");
        assert_eq!(rows[1].created_at, t + TimeDelta::microseconds(1));
        Ok(())
    }

    #[test]
    fn distinct_value_types_at_one_instant_keep_their_timestamp() -> anyhow::Result<()> {
        // Different value_types are already distinct in the natural key — no
        // nudging, so observation chronology stays exactly as observed.
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let mut p = prof("bamboohr", "1", Some("anna@corp.com"), false);
        p.observations = vec![
            input("bamboohr", "1", "id", "1", false, t),
            input("bamboohr", "1", "email", "anna@corp.com", false, t),
        ];

        let rows = assignments_to_rows(
            &[PersonAssignment {
                person_id: Uuid::from_u128(7),
                kind: AssignmentKind::Minted,
                profiles: vec![p],
            }],
            Uuid::nil(),
            &HashMap::new(),
        );

        assert!(
            rows.iter().all(|r| r.created_at == t),
            "distinct value_types do not collide"
        );
        Ok(())
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

        let rows = assignments_to_rows(&[minted, linked], Uuid::from_u128(99), &HashMap::new());
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
        let out = resolve_without_roster(
            group_by_email(profiles),
            &HashMap::new(),
            &HashMap::new(),
            counter(),
        );
        assert_eq!(out.assignments.len(), 1, "one person for the email group");
        assert_eq!(out.minted, 2, "both accounts counted");

        let person = out.assignments[0].person_id;
        let obs_rows = assignments_to_rows(&out.assignments, Uuid::from_u128(99), &HashMap::new());
        assert!(!obs_rows.is_empty());
        assert!(
            obs_rows.iter().all(|r| r.person_id == person),
            "every observation stamped with the one resolved person"
        );
        Ok(())
    }

    /// A fixed instant: nothing about roster minting depends on the clock, and a
    /// literal keeps the rows a test builds comparable.
    fn epoch() -> DateTime {
        chrono::DateTime::UNIX_EPOCH.naive_utc()
    }

    /// An addressless profile whose source states the account's own id — what a
    /// roster emits, and the shape roster minting requires.
    fn rostered(source_type: &str, account_id: &str, closed: bool) -> SeedProfile {
        let mut profile = prof(source_type, account_id, None, closed);
        profile.observations = vec![input(
            source_type,
            account_id,
            BINDING_VALUE_TYPE,
            account_id,
            false,
            epoch(),
        )];
        profile
    }

    #[test]
    fn a_blank_roster_setting_names_no_source() {
        for (case, configured) in [("unset", ""), ("spaces", "   "), ("a tab", "\t")] {
            assert!(
                RosterSource::parse(configured).is_none(),
                "should name no source: {case}"
            );
        }

        let Some(parsed) = RosterSource::parse("  bamboohr  ") else {
            panic!("a named source must parse");
        };
        assert_eq!(
            parsed.name(),
            "bamboohr",
            "the name is trimmed, not rejected"
        );
        assert!(parsed.speaks_for("bamboohr"));
        assert!(!parsed.speaks_for("bamboohr-eu"), "no prefix matching");
    }

    #[test]
    fn the_roster_mints_a_person_for_an_account_with_no_address() {
        let groups = group_by_email(vec![rostered("bamboohr", "e-1", false)]);

        let out = resolve_assignments(
            groups,
            &HashMap::new(),
            &HashMap::new(),
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );

        assert_eq!(out.minted_from_roster, 1);
        assert_eq!(out.skipped_no_email, 0, "the roster vouches for it");
        assert_eq!(out.minted, 0, "counted apart from an address-matched mint");
        assert_eq!(
            out.assignments.iter().map(|a| a.kind).collect::<Vec<_>>(),
            vec![AssignmentKind::MintedFromRoster],
        );
    }

    #[test]
    fn only_the_configured_source_mints_without_an_address() {
        // Two independent singletons, not a mixed group — an addressless profile
        // is always its own group. Every other source keeps needing an address:
        // minting from two rosters gives one addressless human two persons, and
        // nothing joins them after.
        let groups = group_by_email(vec![
            rostered("bamboohr", "e-1", false),
            rostered("zoom", "Z1", false),
        ]);

        let out = resolve_assignments(
            groups,
            &HashMap::new(),
            &HashMap::new(),
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );

        assert_eq!(out.minted_from_roster, 1, "only the bamboohr account");
        assert_eq!(out.skipped_no_email, 1, "the zoom account is still skipped");
    }

    #[test]
    fn no_roster_configured_leaves_every_addressless_account_alone() {
        let groups = group_by_email(vec![rostered("bamboohr", "e-1", false)]);

        let out = resolve_without_roster(groups, &HashMap::new(), &HashMap::new(), counter());

        assert_eq!(out.minted_from_roster, 0);
        assert_eq!(out.skipped_no_email, 1, "the default is the old behaviour");
        assert!(out.assignments.is_empty());
    }

    #[test]
    fn a_closed_roster_account_is_not_minted() {
        // The source has deactivated it, so there is no human to add.
        let groups = group_by_email(vec![rostered("bamboohr", "e-1", true)]);

        let out = resolve_assignments(
            groups,
            &HashMap::new(),
            &HashMap::new(),
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );

        assert_eq!(out.minted_from_roster, 0);
        assert_eq!(out.skipped_closed, 1);
        assert!(out.assignments.is_empty());
    }

    #[test]
    fn a_roster_account_that_states_no_id_is_not_minted() {
        // The `id` observation is what becomes the binding row. Minting without
        // one would leave a person no account points at — invisible to every
        // later run, and to the operator who would have to repair it.
        let mut silent = prof("bamboohr", "e-1", None, false);
        silent.observations = vec![input(
            "bamboohr",
            "e-1",
            "display_name",
            "Sam Example",
            false,
            epoch(),
        )];

        let out = resolve_assignments(
            group_by_email(vec![silent]),
            &HashMap::new(),
            &HashMap::new(),
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );

        assert_eq!(out.minted_from_roster, 0);
        assert_eq!(out.skipped_no_email, 1);
        assert!(out.assignments.is_empty());
    }

    #[test]
    fn a_roster_account_with_an_address_takes_the_address_path() {
        // The roster branch exists for accounts automation cannot match. One
        // carrying an address is matched on it, so it must not be marked as
        // needing an operator's eye.
        let addressed = prof("bamboohr", "e-1", Some("sam@example.com"), false);
        let mut known_person = HashMap::new();
        known_person.insert("sam@example.com".to_owned(), Uuid::from_u128(0x5A_11));

        let out = resolve_assignments(
            group_by_email(vec![addressed]),
            &HashMap::new(),
            &known_person,
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );

        assert_eq!(out.minted_from_roster, 0);
        assert_eq!(out.linked_by_email, 1);
        assert_eq!(out.assignments[0].person_id, Uuid::from_u128(0x5A_11));
    }

    #[test]
    fn an_already_bound_roster_account_is_reused_not_minted_again() {
        // Idempotence: the binding written by the first run is what the second
        // one finds, so a daily seed does not add a person a day.
        let mut known = HashMap::new();
        known.insert(
            SourceAccountKey {
                source_type: "bamboohr".to_owned(),
                source_id: Uuid::from_u128(1),
                account_id: "e-1".to_owned(),
            },
            KnownBinding {
                person_id: Uuid::from_u128(0x5A_11),
                author_person_id: Uuid::nil(),
                provenance: Provenance::RosterMint,
            },
        );

        let out = resolve_assignments(
            group_by_email(vec![rostered("bamboohr", "e-1", false)]),
            &known,
            &HashMap::new(),
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );

        assert_eq!(out.minted_from_roster, 0);
        assert_eq!(out.reused_known, 1);
        assert_eq!(out.assignments[0].person_id, Uuid::from_u128(0x5A_11));
    }

    #[test]
    fn a_roster_mint_is_stamped_so_the_queue_can_find_it() {
        // The reason is the only record that the mint had no address behind it:
        // the binding read turns it back into the item an operator confirms.
        let rows = assignments_to_rows(
            &[PersonAssignment {
                person_id: Uuid::from_u128(7),
                kind: AssignmentKind::MintedFromRoster,
                profiles: vec![rostered("bamboohr", "e-1", false)],
            }],
            Uuid::nil(),
            &HashMap::new(),
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value_type, BINDING_VALUE_TYPE);
        assert_eq!(rows[0].reason.as_deref(), Some(ROSTER_MINT_REASON));
        assert!(
            rows[0].author_person_id.is_nil(),
            "automation, not an operator decision"
        );
    }

    #[test]
    fn every_assignment_kind_stamps_its_own_reason() {
        for (kind, expected) in [
            (AssignmentKind::ReusedKnown, ""),
            (AssignmentKind::Minted, ""),
            (AssignmentKind::LinkedByEmail, AUTO_SEED_LINK_REASON),
            (AssignmentKind::MintedFromRoster, ROSTER_MINT_REASON),
        ] {
            let rows = assignments_to_rows(
                &[PersonAssignment {
                    person_id: Uuid::from_u128(7),
                    kind,
                    profiles: vec![rostered("bamboohr", "e-1", false)],
                }],
                Uuid::nil(),
                &HashMap::new(),
            );

            assert_eq!(
                rows[0].reason.as_deref(),
                Some(expected),
                "wrong reason for {kind:?}"
            );
        }
    }

    fn roster_bound(person: u128) -> KnownBinding {
        KnownBinding {
            person_id: Uuid::from_u128(person),
            author_person_id: Uuid::nil(),
            provenance: Provenance::RosterMint,
        }
    }

    #[test]
    fn an_unconfirmed_mint_keeps_saying_so_when_the_source_changes_the_account() {
        // The source re-emits this account's id row on every change it makes, so
        // the run after a mint writes a NEWER binding row. The queue reads the
        // latest row, so an empty reason there would retire the operator's item
        // with no decision behind it.
        let account = SourceAccountKey {
            source_type: "bamboohr".to_owned(),
            source_id: Uuid::from_u128(1),
            account_id: "e-1".to_owned(),
        };
        let mut known = HashMap::new();
        known.insert(account.clone(), roster_bound(0x5A_11));

        let out = resolve_assignments(
            group_by_email(vec![rostered("bamboohr", "e-1", false)]),
            &known,
            &HashMap::new(),
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );
        let rows = assignments_to_rows(&out.assignments, Uuid::nil(), &known);

        assert_eq!(out.reused_known, 1, "the binding is reused, not re-minted");
        assert!(
            !rows.is_empty(),
            "the account's observations are re-emitted"
        );
        for row in &rows {
            assert_eq!(
                row.reason.as_deref(),
                Some(ROSTER_MINT_REASON),
                "a re-emitted row must not overwrite the mint reason"
            );
        }
    }

    #[test]
    fn an_operators_decision_is_never_re_stamped_as_unconfirmed() {
        let account = SourceAccountKey {
            source_type: "bamboohr".to_owned(),
            source_id: Uuid::from_u128(1),
            account_id: "e-1".to_owned(),
        };
        let mut known = HashMap::new();
        known.insert(
            account.clone(),
            KnownBinding {
                person_id: Uuid::from_u128(0x5A_11),
                author_person_id: Uuid::from_u128(0xAD_1119),
                provenance: Provenance::RosterMint,
            },
        );

        let out = resolve_assignments(
            group_by_email(vec![rostered("bamboohr", "e-1", false)]),
            &known,
            &HashMap::new(),
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );
        let rows = assignments_to_rows(&out.assignments, Uuid::nil(), &known);

        for row in &rows {
            assert_eq!(
                row.reason.as_deref(),
                Some(""),
                "an operator has decided; nothing is owed a second look"
            );
        }
    }

    #[test]
    fn an_address_reclaims_an_account_from_the_person_it_was_minted_for() {
        // The whole point of the roster mint is that nothing could match the
        // account. Once something can, leaving it on the minted person would
        // hand one human two persons AND give the address two claimants — which
        // resolves to nobody downstream, so the human's activity would reach no
        // metric at all.
        let real = Uuid::from_u128(0xBEEF);
        let mut known = HashMap::new();
        known.insert(
            SourceAccountKey {
                source_type: "bamboohr".to_owned(),
                source_id: Uuid::from_u128(1),
                account_id: "e-1".to_owned(),
            },
            roster_bound(0x5A_11),
        );
        let mut email_map = HashMap::new();
        email_map.insert("sam@example.com".to_owned(), real);

        // The roster has published the address the other account already holds.
        let mut addressed = rostered("bamboohr", "e-1", false);
        addressed.latest_email = Some("sam@example.com".to_owned());

        let out = resolve_assignments(
            group_by_email(vec![addressed]),
            &known,
            &email_map,
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );
        let rows = assignments_to_rows(&out.assignments, Uuid::nil(), &known);

        assert_eq!(out.linked_by_email, 1, "the address decides now");
        assert_eq!(out.reused_known, 0, "the unconfirmed mint gave way");
        assert_eq!(out.assignments[0].person_id, real);
        for row in &rows {
            assert_eq!(
                row.reason.as_deref(),
                Some(AUTO_SEED_LINK_REASON),
                "the link is the resolution; it is not still unconfirmed"
            );
        }
    }

    #[test]
    fn an_address_does_not_override_a_decision_a_human_made() {
        let mut known = HashMap::new();
        known.insert(
            SourceAccountKey {
                source_type: "bamboohr".to_owned(),
                source_id: Uuid::from_u128(1),
                account_id: "e-1".to_owned(),
            },
            operator_bound(0x5A_11),
        );
        let mut email_map = HashMap::new();
        email_map.insert("sam@example.com".to_owned(), Uuid::from_u128(0x0B_11));

        let mut addressed = rostered("bamboohr", "e-1", false);
        addressed.latest_email = Some("sam@example.com".to_owned());

        let out = resolve_assignments(
            group_by_email(vec![addressed]),
            &known,
            &email_map,
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );

        assert_eq!(out.reused_known, 1);
        assert_eq!(out.linked_by_email, 0, "an operator's decision is final");
        assert_eq!(out.assignments[0].person_id, Uuid::from_u128(0x5A_11));
    }

    #[test]
    fn an_excluded_roster_account_is_never_minted_a_person() {
        // An operator has declared this account not a human. Minting for it
        // would undo that decision every night, and automation may not spread
        // an exclusion either — so it simply leaves before any decision.
        let mut known = HashMap::new();
        known.insert(
            SourceAccountKey {
                source_type: "bamboohr".to_owned(),
                source_id: Uuid::from_u128(1),
                account_id: "svc-1".to_owned(),
            },
            KnownBinding {
                person_id: EXCLUDED_PERSON,
                author_person_id: Uuid::from_u128(0xAD_1119),
                provenance: Provenance::Resolved,
            },
        );

        let out = resolve_assignments(
            group_by_email(vec![rostered("bamboohr", "svc-1", false)]),
            &known,
            &HashMap::new(),
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );

        assert_eq!(out.skipped_excluded, 1);
        assert_eq!(out.minted_from_roster, 0);
        assert!(out.assignments.is_empty(), "nothing is re-emitted for it");
    }

    #[test]
    fn an_id_too_long_to_store_is_not_a_licence_to_mint() {
        // `route_value` drops an over-long id rather than truncating it, so the
        // binding row would never be written and the person would be reachable
        // from no account — and with no address, every later run mints another.
        let mut oversized = prof("bamboohr", "e-1", None, false);
        let too_long = "x".repeat(321);
        oversized.observations = vec![input(
            "bamboohr",
            "e-1",
            BINDING_VALUE_TYPE,
            &too_long,
            false,
            epoch(),
        )];

        let out = resolve_assignments(
            group_by_email(vec![oversized]),
            &HashMap::new(),
            &HashMap::new(),
            RosterSource::parse("bamboohr").as_ref(),
            counter(),
        );

        assert_eq!(out.minted_from_roster, 0);
        assert_eq!(out.skipped_no_email, 1);
    }
}
