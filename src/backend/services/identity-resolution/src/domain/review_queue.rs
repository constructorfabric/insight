//! What needs an operator decision — derived, never stored.
//!
//! Two inputs joined on the account key: the folded connector evidence (every
//! observed account) and the current bindings from the journal. An item exists
//! only while its condition holds, so a decision removes it without any item
//! lifecycle to maintain.

use std::collections::HashMap;

use uuid::Uuid;

use super::resolution::EXCLUDED_PERSON;
use super::seed::{KnownBinding, SourceAccountKey, normalize_email};

/// Why an account is on the queue. The discriminant orders the queue: a
/// contested account is the operator's most actionable item, a no-evidence one
/// the least.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// The account's identity evidence is claimed by more than one person.
    Contested,
    /// The account's identity value is shared by accounts bound to different
    /// persons, with no operator decision explaining the divergence.
    BindingConflict,
    /// The account carries no identity evidence automation can match on —
    /// e-mail is the only matching key today, so a username-only account is
    /// here too (shown with its username). Visible, never hidden: nothing
    /// will ever bind these but an operator.
    NoEvidence,
}

/// One account awaiting a decision, with the persons it could belong to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub kind: ItemKind,
    pub account: SourceAccountKey,
    pub email: Option<String>,
    pub username: Option<String>,
    pub candidates: Vec<Uuid>,
}

/// How many observed accounts are in each resolution state — the operator-
/// visible match rate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResolutionRates {
    pub observed: usize,
    pub bound: usize,
    pub pending: usize,
    pub no_evidence: usize,
    pub excluded: usize,
}

/// The queue plus its rates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Review {
    pub items: Vec<QueueItem>,
    pub rates: ResolutionRates,
}

/// An account as the evidence describes it — the shape [`build`] consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceAccount {
    pub account: SourceAccountKey,
    pub email: Option<String>,
    pub username: Option<String>,
    pub is_closed: bool,
}

/// Build the review from folded evidence and current bindings.
///
/// Closed accounts are skipped entirely: their latest evidence event is a
/// closure, so there is nothing to decide. Everything else is classified in one
/// pass over the accounts sharing each e-mail.
#[must_use]
pub fn build(
    evidence: Vec<EvidenceAccount>,
    bindings: &HashMap<SourceAccountKey, KnownBinding>,
) -> Review {
    let active: Vec<EvidenceAccount> = evidence.into_iter().filter(|e| !e.is_closed).collect();

    let mut by_email: HashMap<String, Vec<&EvidenceAccount>> = HashMap::new();
    for account in &active {
        if let Some(email) = account.email.as_deref().map(normalize_email) {
            by_email.entry(email).or_default().push(account);
        }
    }

    let mut items = Vec::new();
    let mut rates = ResolutionRates {
        observed: active.len(),
        ..ResolutionRates::default()
    };

    for account in &active {
        match bindings.get(&account.account) {
            Some(binding) if binding.person_id == EXCLUDED_PERSON => rates.excluded += 1,
            Some(_) => rates.bound += 1,
            None if !has_matchable_evidence(account) => {
                rates.no_evidence += 1;
                items.push(item(ItemKind::NoEvidence, account, Vec::new()));
            }
            None => {
                let candidates = candidates_for(account, &by_email, bindings);
                if candidates.len() > 1 {
                    rates.pending += 1;
                    items.push(item(ItemKind::Contested, account, candidates));
                } else {
                    // One candidate or none: the account has an e-mail, so
                    // automation will bind it on its next run — not the
                    // operator's problem yet.
                    rates.pending += 1;
                }
            }
        }
    }

    items.extend(binding_conflicts(&by_email, bindings));

    // The queue is truncated by the caller, so its order must not depend on
    // hash iteration: the same data must surface the same items every build.
    items.sort_by(|a, b| {
        (
            a.kind as u8,
            &a.account.source_type,
            a.account.source_id,
            &a.account.account_id,
        )
            .cmp(&(
                b.kind as u8,
                &b.account.source_type,
                b.account.source_id,
                &b.account.account_id,
            ))
    });

    Review { items, rates }
}

/// Accounts sharing an e-mail whose bindings disagree, with no operator-authored
/// binding explaining the split.
fn binding_conflicts(
    by_email: &HashMap<String, Vec<&EvidenceAccount>>,
    bindings: &HashMap<SourceAccountKey, KnownBinding>,
) -> Vec<QueueItem> {
    let mut conflicts = Vec::new();

    for group in by_email.values() {
        let bound: Vec<KnownBinding> = group
            .iter()
            .filter_map(|a| bindings.get(&a.account).copied())
            .filter(|b| b.person_id != EXCLUDED_PERSON)
            .collect();

        let mut persons: Vec<Uuid> = bound.iter().map(|b| b.person_id).collect();
        persons.sort();
        persons.dedup();
        if persons.len() < 2 || bound.iter().any(KnownBinding::is_operator_authored) {
            continue; // consistent, or a human already settled the split
        }

        for account in group {
            conflicts.push(item(ItemKind::BindingConflict, account, persons.clone()));
        }
    }

    conflicts
}

/// Persons currently claiming any of the account's identity values.
fn candidates_for(
    account: &EvidenceAccount,
    by_email: &HashMap<String, Vec<&EvidenceAccount>>,
    bindings: &HashMap<SourceAccountKey, KnownBinding>,
) -> Vec<Uuid> {
    let Some(email) = account.email.as_deref().map(normalize_email) else {
        return Vec::new();
    };
    let Some(group) = by_email.get(&email) else {
        return Vec::new();
    };

    let mut persons: Vec<Uuid> = group
        .iter()
        .filter_map(|a| bindings.get(&a.account))
        .map(|b| b.person_id)
        .filter(|p| *p != EXCLUDED_PERSON)
        .collect();
    persons.sort();
    persons.dedup();
    persons
}

// A username is displayed but cannot match anyone (the seed links by e-mail
// only), so counting it as evidence would leave a username-only account
// invisible forever: pending, never surfaced, never auto-bound.
fn has_matchable_evidence(account: &EvidenceAccount) -> bool {
    account.email.is_some()
}

fn item(kind: ItemKind, account: &EvidenceAccount, candidates: Vec<Uuid>) -> QueueItem {
    QueueItem {
        kind,
        account: account.account.clone(),
        email: account.email.clone(),
        username: account.username.clone(),
        candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(source_type: &str, id: &str) -> SourceAccountKey {
        SourceAccountKey {
            source_type: source_type.to_owned(),
            source_id: Uuid::from_u128(1),
            account_id: id.to_owned(),
        }
    }

    fn observed(source_type: &str, id: &str, email: Option<&str>) -> EvidenceAccount {
        EvidenceAccount {
            account: account(source_type, id),
            email: email.map(str::to_owned),
            username: None,
            is_closed: false,
        }
    }

    fn seed_bound(person: u128) -> KnownBinding {
        KnownBinding {
            person_id: Uuid::from_u128(person),
            author_person_id: Uuid::nil(),
        }
    }

    fn operator_bound(person: u128) -> KnownBinding {
        KnownBinding {
            person_id: Uuid::from_u128(person),
            author_person_id: Uuid::from_u128(0xAD_1119),
        }
    }

    #[test]
    fn accounts_without_identity_evidence_are_surfaced_not_hidden() {
        let review = build(vec![observed("jira", "jr-1", None)], &HashMap::new());

        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].kind, ItemKind::NoEvidence);
        assert_eq!(review.rates.no_evidence, 1);
        assert_eq!(review.rates.observed, 1);
    }

    #[test]
    fn a_username_only_account_is_surfaced_because_nothing_can_match_it() {
        // A username is shown to the operator but the seed links by e-mail
        // only: were the username counted as evidence, this account would sit
        // in `pending` forever with no queue item and no automation to come.
        let mut orphan = observed("github", "gh-1", None);
        orphan.username = Some("octocat".to_owned());

        let review = build(vec![orphan], &HashMap::new());

        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].kind, ItemKind::NoEvidence);
        assert_eq!(review.items[0].username.as_deref(), Some("octocat"));
        assert_eq!(review.rates.no_evidence, 1);
        assert_eq!(review.rates.pending, 0);
    }

    #[test]
    fn an_unbound_account_with_a_contested_email_lists_its_candidates() {
        let anna = observed("github", "gh-1", Some("team@example.com"));
        let boris = observed("jira", "jr-1", Some("team@example.com"));
        let newcomer = observed("slack", "slk-1", Some("TEAM@example.com"));
        let mut bindings = HashMap::new();
        bindings.insert(anna.account.clone(), seed_bound(5));
        bindings.insert(boris.account.clone(), operator_bound(6));

        let review = build(vec![anna, boris, newcomer.clone()], &bindings);

        let contested: Vec<&QueueItem> = review
            .items
            .iter()
            .filter(|i| i.kind == ItemKind::Contested)
            .collect();
        assert_eq!(contested.len(), 1, "only the unbound newcomer is pending");
        assert_eq!(contested[0].account, newcomer.account);
        assert_eq!(
            contested[0].candidates,
            vec![Uuid::from_u128(5), Uuid::from_u128(6)]
        );
        assert_eq!(review.rates.bound, 2);
        assert_eq!(review.rates.pending, 1);
    }

    #[test]
    fn operator_settled_divergence_is_not_a_conflict() {
        let anna = observed("github", "gh-1", Some("team@example.com"));
        let boris = observed("jira", "jr-1", Some("team@example.com"));
        let mut bindings = HashMap::new();
        bindings.insert(anna.account.clone(), seed_bound(5));
        bindings.insert(boris.account.clone(), operator_bound(6));

        let review = build(vec![anna, boris], &bindings);

        assert!(
            !review
                .items
                .iter()
                .any(|i| i.kind == ItemKind::BindingConflict),
            "a human already settled this split"
        );
    }

    #[test]
    fn all_seed_divergence_surfaces_as_a_binding_conflict() {
        let a = observed("github", "gh-1", Some("legacy@example.com"));
        let b = observed("jira", "jr-1", Some("legacy@example.com"));
        let mut bindings = HashMap::new();
        bindings.insert(a.account.clone(), seed_bound(17));
        bindings.insert(b.account.clone(), seed_bound(23));

        let review = build(vec![a, b], &bindings);

        let conflicts: Vec<&QueueItem> = review
            .items
            .iter()
            .filter(|i| i.kind == ItemKind::BindingConflict)
            .collect();
        assert_eq!(conflicts.len(), 2, "both accounts of the group are shown");
        assert_eq!(
            conflicts[0].candidates,
            vec![Uuid::from_u128(17), Uuid::from_u128(23)]
        );
    }

    #[test]
    fn queue_order_is_stable_across_builds() {
        let a = observed("github", "gh-1", Some("legacy@example.com"));
        let b = observed("jira", "jr-1", Some("legacy@example.com"));
        let orphan = observed("slack", "slk-9", None);
        let mut bindings = HashMap::new();
        bindings.insert(a.account.clone(), seed_bound(17));
        bindings.insert(b.account.clone(), seed_bound(23));

        let first = build(vec![a.clone(), b.clone(), orphan.clone()], &bindings);
        let again = build(vec![orphan, b, a], &bindings);

        assert_eq!(
            first.items, again.items,
            "input order must not change what the operator sees"
        );
    }

    #[test]
    fn closed_accounts_leave_the_queue() {
        let mut closed = observed("github", "gh-9", None);
        closed.is_closed = true;

        let review = build(vec![closed], &HashMap::new());

        assert!(
            review.items.is_empty(),
            "a closed account needs no decision"
        );
        assert_eq!(review.rates.observed, 0);
    }

    #[test]
    fn excluded_accounts_are_counted_but_never_shown() {
        let bot = observed("github", "gh-bot", Some("ci@example.com"));
        let mut bindings = HashMap::new();
        bindings.insert(
            bot.account.clone(),
            KnownBinding {
                person_id: EXCLUDED_PERSON,
                author_person_id: Uuid::from_u128(0xAD_1119),
            },
        );

        let review = build(vec![bot], &bindings);

        assert!(review.items.is_empty());
        assert_eq!(review.rates.excluded, 1);
        assert_eq!(review.rates.bound, 0, "excluded is its own state");
    }
}
