//! What needs an operator decision — derived, never stored.
//!
//! Two inputs joined on the account key: the folded connector evidence (every
//! observed account) and the current bindings from the journal. An item exists
//! only while its condition holds, so a decision removes it without any item
//! lifecycle to maintain.

use std::collections::HashMap;

use uuid::Uuid;

use super::provenance::Provenance;
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
    /// The binding was minted during a sign-in so its owner could get in. The
    /// person may duplicate one the roster already knows — nothing could join
    /// them, since the account carries no address — and only an operator can
    /// say whether it is its own person or the same human.
    ProvisionedAtLogin,
    /// The batch minted a person for this account because the roster lists it,
    /// not because anything matched: the account carries no address. The person
    /// may duplicate one already on the roster under a different account, and
    /// only an operator can say which it is.
    MintedFromRoster,
    /// No route to a person exists for the account: no connector states an id
    /// to write a binding from, and nobody holds its address for the seed to
    /// fold it into. Only an operator, or a sign-in through the account, can
    /// decide it — which is why it belongs on the queue rather than in
    /// `pending`. An id-less account whose address IS held stays in `pending`:
    /// the seed attaches it to that person, and its activity resolves through
    /// the binding that person already has.
    NoSourceId,
    /// The account carries no identity evidence automation can match on —
    /// e-mail is the only matching key today, so a username-only account is
    /// here too (shown with its username). Visible, never hidden: nothing
    /// will ever bind these but an operator.
    NoEvidence,
}

/// What the connector says this account IS, beyond the values automation
/// matches on.
///
/// The matcher needs an address and nothing else, so the fold used to read
/// nothing else — which left the accounts it cannot match displayed as a bare
/// id, exactly the ones only a human can bind. The source usually describes
/// them perfectly well; a person recognises a name, a job title and a manager.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountDescription {
    pub display_name: Option<String>,
    pub job_title: Option<String>,
    pub department: Option<String>,
    /// Employment status as the source words it — a leaver is rarely worth an
    /// operator's attention, and the queue should not hide that they are one.
    pub status: Option<String>,
    pub manager_email: Option<String>,
}

/// One account awaiting a decision, with the persons it could belong to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub kind: ItemKind,
    pub account: SourceAccountKey,
    pub email: Option<String>,
    pub username: Option<String>,
    pub description: AccountDescription,
    /// The person holding this account right now, when one does. Absent means
    /// unbound — which is itself the answer for a contested account.
    pub bound_to: Option<Uuid>,
    pub candidates: Vec<Uuid>,
}

/// How many observed accounts are in each resolution state — the operator-
/// visible match rate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResolutionRates {
    pub observed: usize,
    pub bound: usize,
    pub pending: usize,
    /// Unbound accounts no seed run can bind — an operator decision is the only
    /// way out. Counted apart from `pending`, which promises the opposite.
    pub no_source_id: usize,
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
    pub description: AccountDescription,
    pub is_closed: bool,
    /// A connector states an account id for it — the seed's only route to a
    /// binding. See [`AccountEvidence::states_binding_id`].
    ///
    /// [`AccountEvidence::states_binding_id`]: crate::infra::identity_evidence::AccountEvidence::states_binding_id
    pub states_binding_id: bool,
}

/// Build the review from folded evidence, current bindings, and the addresses
/// the persons journal already names an owner for.
///
/// `claimed_addresses` is the seed's own `email -> person` map, passed in rather
/// than re-derived: it is the second of the two routes automation has to a
/// person, and it reaches further than the evidence does — an address stays
/// claimed after the account that carried it closes or leaves the roster.
/// Deriving it from evidence instead would queue accounts the next seed run
/// resolves by itself.
///
/// Closed accounts are skipped entirely: their latest evidence event is a
/// closure, so there is nothing to decide. Everything else is classified in one
/// pass over the accounts sharing each e-mail.
#[must_use]
pub fn build(
    evidence: Vec<EvidenceAccount>,
    bindings: &HashMap<SourceAccountKey, KnownBinding>,
    claimed_addresses: &HashMap<String, Uuid>,
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
            Some(binding) => {
                rates.bound += 1;
                items.extend(unconfirmed_item(account, binding));
            }
            None if !has_matchable_evidence(account) => {
                rates.no_evidence += 1;
                items.push(item(ItemKind::NoEvidence, account, None, Vec::new()));
            }
            None => {
                let candidates = candidates_for(account, &by_email, bindings);
                if candidates.len() > 1 {
                    rates.pending += 1;
                    items.push(item(ItemKind::Contested, account, None, candidates));
                } else if !account.states_binding_id
                    && candidates.is_empty()
                    && !address_is_claimed(account, claimed_addresses)
                {
                    // Neither route to a person exists: the source states no id
                    // to write a binding from, and nobody holds the address to
                    // fold it into. Every later run answers the same, so an
                    // operator is the only way out.
                    rates.no_source_id += 1;
                    items.push(item(ItemKind::NoSourceId, account, None, candidates));
                } else {
                    // Automation still has a route: either the source states an
                    // id it can bind, or one person already holds the address and
                    // the seed folds this account into them. Not the operator's
                    // problem yet.
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
            let bound_to = bindings.get(&account.account).map(|b| b.person_id);
            conflicts.push(item(
                ItemKind::BindingConflict,
                account,
                bound_to,
                persons.clone(),
            ));
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

/// Whether the persons journal already names an owner for this account's
/// address — the seed's second route to a person (`resolve_assignments` step 2,
/// its `claimed_by`).
///
/// INVARIANT: the lookup mirrors that one exactly — same normalization, the
/// same map as handed to the seed, and the same sentinel exclusion. Answering
/// it differently here would either hide an account the seed skips or queue one
/// the seed resolves; both are states nobody can see from the other side.
fn address_is_claimed(account: &EvidenceAccount, claimed: &HashMap<String, Uuid>) -> bool {
    account
        .email
        .as_deref()
        .map(normalize_email)
        .and_then(|email| claimed.get(&email).copied())
        .is_some_and(|person| person != EXCLUDED_PERSON)
}

/// The item an already-bound account earns, when its binding still awaits a
/// human. An operator's own decision never does, whatever wrote the row before
/// it: re-asserting a binding IS the confirm act.
fn unconfirmed_item(account: &EvidenceAccount, binding: &KnownBinding) -> Option<QueueItem> {
    if binding.is_operator_authored() {
        return None;
    }

    // Both trade an unusable account for a possibly duplicate person: a login
    // mint so its owner could get in, a roster mint so the roster is complete.
    // Neither trade is closed until a human says which person it is.
    let kind = match binding.provenance {
        Provenance::Resolved => return None,
        Provenance::LoginBootstrap => ItemKind::ProvisionedAtLogin,
        Provenance::RosterMint => ItemKind::MintedFromRoster,
    };

    Some(item(
        kind,
        account,
        Some(binding.person_id),
        vec![binding.person_id],
    ))
}

fn item(
    kind: ItemKind,
    account: &EvidenceAccount,
    bound_to: Option<Uuid>,
    candidates: Vec<Uuid>,
) -> QueueItem {
    QueueItem {
        kind,
        account: account.account.clone(),
        email: account.email.clone(),
        username: account.username.clone(),
        description: account.description.clone(),
        bound_to,
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

    /// The ordinary case: a connector states an id for the account, so the seed
    /// can bind it. The accounts that state none are built by
    /// [`without_source_id`].
    fn observed(source_type: &str, id: &str, email: Option<&str>) -> EvidenceAccount {
        EvidenceAccount {
            account: account(source_type, id),
            email: email.map(str::to_owned),
            username: None,
            description: AccountDescription::default(),
            is_closed: false,
            states_binding_id: true,
        }
    }

    fn without_source_id(source_type: &str, id: &str, email: Option<&str>) -> EvidenceAccount {
        EvidenceAccount {
            states_binding_id: false,
            ..observed(source_type, id, email)
        }
    }

    fn described(source_type: &str, id: &str, display_name: &str) -> EvidenceAccount {
        EvidenceAccount {
            description: AccountDescription {
                display_name: Some(display_name.to_owned()),
                job_title: Some("Engineer".to_owned()),
                ..AccountDescription::default()
            },
            ..observed(source_type, id, None)
        }
    }

    fn seed_bound(person: u128) -> KnownBinding {
        KnownBinding {
            person_id: Uuid::from_u128(person),
            author_person_id: Uuid::nil(),
            provenance: Provenance::Resolved,
        }
    }

    fn login_bound(person: u128) -> KnownBinding {
        KnownBinding {
            person_id: Uuid::from_u128(person),
            author_person_id: Uuid::nil(),
            provenance: Provenance::LoginBootstrap,
        }
    }

    /// An operator's own decision. Its provenance is `Resolved` because a verb
    /// appends a NEW row carrying `operator-bind`, never a mint reason — so this
    /// is what the binding read actually returns after a confirm.
    fn operator_bound(person: u128) -> KnownBinding {
        KnownBinding {
            person_id: Uuid::from_u128(person),
            author_person_id: Uuid::from_u128(0xAD_1119),
            provenance: Provenance::Resolved,
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
    fn a_roster_mint_asks_the_operator_whose_person_it_is() {
        // The roster said a human exists and the batch gave them a person so the
        // organisation is complete. Whether that person is already on the roster
        // under another account is a question only a human can answer.
        let mut bindings = HashMap::new();
        bindings.insert(account("bamboohr", "e-1"), roster_bound(7));

        let review = build(
            vec![described("bamboohr", "e-1", "Sam Example")],
            &bindings,
            &HashMap::new(),
        );

        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].kind, ItemKind::MintedFromRoster);
        assert_eq!(review.items[0].bound_to, Some(Uuid::from_u128(7)));
        assert_eq!(
            review.items[0].candidates,
            vec![Uuid::from_u128(7)],
            "the minted person is the one being confirmed"
        );
        assert_eq!(
            review.items[0].description.display_name.as_deref(),
            Some("Sam Example")
        );
        assert_eq!(review.rates.bound, 1);
        assert_eq!(review.rates.no_evidence, 0, "bound, so not unmatched");
    }

    #[test]
    fn the_authorship_guard_outranks_any_provenance() {
        // Defensive, and deliberately a state no writer produces today: a verb
        // appends `operator-bind`, so an operator-authored row never carries a
        // mint reason. Pinning the precedence means a future writer that does
        // produce this pair cannot resurrect a decided account into the queue.
        let mut bindings = HashMap::new();
        bindings.insert(
            account("bamboohr", "e-1"),
            KnownBinding {
                person_id: Uuid::from_u128(7),
                author_person_id: Uuid::from_u128(0xAD_1119),
                provenance: Provenance::RosterMint,
            },
        );

        let review = build(
            vec![observed("bamboohr", "e-1", None)],
            &bindings,
            &HashMap::new(),
        );

        assert!(
            review.items.is_empty(),
            "a human has said whose it is; nothing left to ask"
        );
    }

    #[test]
    fn a_binding_the_resolver_matched_asks_nothing() {
        let mut bindings = HashMap::new();
        bindings.insert(account("bamboohr", "e-1"), seed_bound(7));

        let review = build(
            vec![observed("bamboohr", "e-1", Some("sam@example.com"))],
            &bindings,
            &HashMap::new(),
        );

        assert!(
            review.items.is_empty(),
            "an address is the evidence; there is nothing to confirm"
        );
    }

    #[test]
    fn the_queue_orders_the_operators_work_before_the_unanswerable() {
        // The discriminant is the sort key, so this pins the working order a
        // reordered enum would silently change.
        let contested_a = observed("github", "gh-1", Some("shared@example.com"));
        let contested_b = observed("jira", "jr-1", Some("shared@example.com"));
        // Unbound, and its address is claimed by both of the above: the one kind
        // the enum ranks first, and the one a five-kind order test cannot omit.
        let newcomer = observed("slack", "slk-1", Some("shared@example.com"));
        let login = observed("github", "gh-9", None);
        let roster = observed("bamboohr", "e-1", None);
        // Addressed, unbound, and its source states no id: the seed can never
        // bind it, so it ranks above the accounts nothing describes at all.
        let unbindable = without_source_id("gitlab", "gl-1", Some("solo@example.com"));
        let orphan = observed("zoom", "z-1", None);

        let mut bindings = HashMap::new();
        bindings.insert(contested_a.account.clone(), seed_bound(17));
        bindings.insert(contested_b.account.clone(), seed_bound(23));
        bindings.insert(login.account.clone(), login_bound(31));
        bindings.insert(roster.account.clone(), roster_bound(37));

        let review = build(
            vec![
                contested_a,
                contested_b,
                newcomer,
                login,
                roster,
                unbindable,
                orphan,
            ],
            &bindings,
            &HashMap::new(),
        );

        let kinds: Vec<ItemKind> = review.items.iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ItemKind::Contested,
                // Three conflicts for two bound accounts: the conflict pass
                // shows every account of the divergent group, the newcomer
                // included, so ONE account can hold two items. Anything
                // comparing the queue's length against the rates has to survive
                // that — the rates count each account once.
                ItemKind::BindingConflict,
                ItemKind::BindingConflict,
                ItemKind::BindingConflict,
                ItemKind::ProvisionedAtLogin,
                ItemKind::MintedFromRoster,
                ItemKind::NoSourceId,
                ItemKind::NoEvidence,
            ],
        );
    }

    #[test]
    fn an_account_whose_source_states_no_id_is_queued_not_left_to_automation() {
        // `pending` promises the next seed run will bind it. Nothing can: the
        // binding row is written from a source-stated id, and this account has
        // none — so leaving it there hides it from the only party who can
        // resolve it, forever.
        let review = build(
            vec![without_source_id(
                "github-commit-email",
                "sam@corp.com",
                Some("sam@corp.com"),
            )],
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].kind, ItemKind::NoSourceId);
        assert_eq!(review.rates.no_source_id, 1);
        assert_eq!(review.rates.pending, 0, "it is not waiting on automation");
    }

    #[test]
    fn a_source_stated_id_keeps_an_unbound_account_off_the_queue() {
        // The counterpart: the seed WILL bind this one, so showing it would ask
        // the operator for a decision automation is about to make.
        let review = build(
            vec![observed("bamboohr", "e-1", Some("sam@corp.com"))],
            &HashMap::new(),
            &HashMap::new(),
        );

        assert!(review.items.is_empty());
        assert_eq!(review.rates.pending, 1);
        assert_eq!(review.rates.no_source_id, 0);
    }

    #[test]
    fn an_id_less_account_whose_address_is_already_held_stays_with_automation() {
        // The seed folds it into the person holding the address, and its
        // activity resolves through that person's own binding — so showing it
        // would ask an operator to confirm what automation already did, forever.
        let held = observed("bamboohr", "e-1", Some("sam@corp.com"));
        let claim = without_source_id("github-commit-email", "sam@corp.com", Some("sam@corp.com"));

        let mut bindings = HashMap::new();
        bindings.insert(held.account.clone(), seed_bound(17));

        let review = build(vec![held, claim], &bindings, &HashMap::new());

        assert!(review.items.is_empty(), "got {:?}", review.items);
        assert_eq!(review.rates.no_source_id, 0);
        assert_eq!(review.rates.pending, 1, "the claim waits on the next run");
    }

    #[test]
    fn an_address_the_journal_already_owns_keeps_its_account_with_automation() {
        // The account that carried this address has closed, so it is out of the
        // evidence and no candidate can be derived from it — but the journal
        // still names its person, and the seed attaches this new account to them
        // on its next run. Deriving the answer from evidence alone would queue
        // an account automation resolves by itself, and keep queueing it.
        let claim = without_source_id("github-commit-email", "sam@corp.com", Some("Sam@Corp.com"));
        let mut claimed = HashMap::new();
        claimed.insert("sam@corp.com".to_owned(), Uuid::from_u128(17));

        let review = build(vec![claim], &HashMap::new(), &claimed);

        assert!(review.items.is_empty(), "got {:?}", review.items);
        assert_eq!(review.rates.no_source_id, 0);
        assert_eq!(review.rates.pending, 1);
    }

    #[test]
    fn an_address_held_only_by_the_excluded_sentinel_is_held_by_nobody() {
        // An exclusion is not a person: the seed drops such an account before
        // any linking decision, so it reaches the mint branch, which now skips
        // it — the account really does need an operator.
        let claim = without_source_id("github-commit-email", "bot@corp.com", Some("bot@corp.com"));
        let mut claimed = HashMap::new();
        claimed.insert("bot@corp.com".to_owned(), EXCLUDED_PERSON);

        let review = build(vec![claim], &HashMap::new(), &claimed);

        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].kind, ItemKind::NoSourceId);
    }

    #[test]
    fn a_contested_address_outranks_a_missing_source_id() {
        // Both conditions hold; the operator needs the more specific one — which
        // person claims it — and the candidate list that comes with it.
        let a = observed("github", "gh-1", Some("shared@example.com"));
        let b = observed("jira", "jr-1", Some("shared@example.com"));
        let claim = without_source_id(
            "github-commit-email",
            "shared@example.com",
            Some("shared@example.com"),
        );

        let mut bindings = HashMap::new();
        bindings.insert(a.account.clone(), seed_bound(17));
        bindings.insert(b.account.clone(), seed_bound(23));

        let review = build(vec![a, b, claim.clone()], &bindings, &HashMap::new());

        let claim_items: Vec<ItemKind> = review
            .items
            .iter()
            .filter(|i| i.account == claim.account)
            .map(|i| i.kind)
            .collect();
        assert!(
            claim_items.contains(&ItemKind::Contested),
            "expected the contested kind for the claim-only account, got {claim_items:?}"
        );
        assert_eq!(review.rates.no_source_id, 0);
    }

    #[test]
    fn accounts_without_identity_evidence_are_surfaced_not_hidden() {
        let review = build(
            vec![observed("jira", "jr-1", None)],
            &HashMap::new(),
            &HashMap::new(),
        );

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

        let review = build(vec![orphan], &HashMap::new(), &HashMap::new());

        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].kind, ItemKind::NoEvidence);
        assert_eq!(review.items[0].username.as_deref(), Some("octocat"));
        assert_eq!(review.rates.no_evidence, 1);
        assert_eq!(review.rates.pending, 0);
    }

    #[test]
    fn a_no_evidence_item_carries_what_the_source_says_the_account_is() {
        // Nothing here is matchable, which is the point: these accounts are
        // bound by a person or by nobody, and a person needs to recognise one.
        let review = build(
            vec![described("bamboohr", "921", "Ann Lee")],
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(review.items[0].kind, ItemKind::NoEvidence);
        assert_eq!(
            review.items[0].description.display_name.as_deref(),
            Some("Ann Lee")
        );
        assert_eq!(
            review.items[0].description.job_title.as_deref(),
            Some("Engineer")
        );
    }

    #[test]
    fn an_unbound_account_with_a_contested_email_lists_its_candidates() {
        let anna = observed("github", "gh-1", Some("team@example.com"));
        let boris = observed("jira", "jr-1", Some("team@example.com"));
        let newcomer = observed("slack", "slk-1", Some("TEAM@example.com"));
        let mut bindings = HashMap::new();
        bindings.insert(anna.account.clone(), seed_bound(5));
        bindings.insert(boris.account.clone(), operator_bound(6));

        let review = build(
            vec![anna, boris, newcomer.clone()],
            &bindings,
            &HashMap::new(),
        );

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

        let review = build(vec![anna, boris], &bindings, &HashMap::new());

        assert!(
            !review
                .items
                .iter()
                .any(|i| i.kind == ItemKind::BindingConflict),
            "a human already settled this split"
        );
    }

    #[test]
    fn a_conflict_row_names_the_person_holding_that_account() {
        // The candidates are the same for every row of the case; which of them
        // holds THIS account is the fact each decision turns on.
        let mut bindings = HashMap::new();
        bindings.insert(account("hr", "1"), seed_bound(1));
        bindings.insert(account("chat", "2"), seed_bound(2));

        let review = build(
            vec![
                observed("hr", "1", Some("a@example.com")),
                observed("chat", "2", Some("a@example.com")),
            ],
            &bindings,
            &HashMap::new(),
        );

        let hr: Vec<&QueueItem> = review
            .items
            .iter()
            .filter(|i| i.account.source_type == "hr")
            .collect();

        assert_eq!(hr.len(), 1, "one row for the hr account");
        assert_eq!(hr[0].kind, ItemKind::BindingConflict);
        assert_eq!(hr[0].bound_to, Some(Uuid::from_u128(1)));
        assert_eq!(hr[0].candidates.len(), 2, "both sides stay listed");
    }

    #[test]
    fn a_login_minted_binding_waits_for_a_human_to_say_whose_it_is() {
        // It IS bound — that is the point, its owner can sign in — so it counts
        // as resolved while still needing a decision.
        let mut bindings = HashMap::new();
        bindings.insert(account("github", "gh-1"), login_bound(7));

        let review = build(
            vec![observed("github", "gh-1", None)],
            &bindings,
            &HashMap::new(),
        );

        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].kind, ItemKind::ProvisionedAtLogin);
        assert_eq!(review.items[0].bound_to, Some(Uuid::from_u128(7)));
        assert_eq!(
            review.items[0].candidates,
            vec![Uuid::from_u128(7)],
            "the minted person is the one being confirmed"
        );
        assert_eq!(review.rates.bound, 1);
        assert_eq!(review.rates.no_evidence, 0, "bound, so not unmatched");
    }

    #[test]
    fn an_operator_decision_retires_the_login_mint() {
        let mut bindings = HashMap::new();
        bindings.insert(
            account("github", "gh-1"),
            KnownBinding {
                person_id: Uuid::from_u128(7),
                author_person_id: Uuid::from_u128(99),
                provenance: Provenance::LoginBootstrap,
            },
        );

        let review = build(
            vec![observed("github", "gh-1", None)],
            &bindings,
            &HashMap::new(),
        );

        assert!(
            review.items.is_empty(),
            "a human has said whose it is; nothing left to ask"
        );
    }

    #[test]
    fn an_unbound_queue_item_is_bound_to_nobody() {
        let review = build(
            vec![observed("jira", "jr-1", None)],
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(review.items[0].bound_to, None);
    }

    #[test]
    fn all_seed_divergence_surfaces_as_a_binding_conflict() {
        let a = observed("github", "gh-1", Some("legacy@example.com"));
        let b = observed("jira", "jr-1", Some("legacy@example.com"));
        let mut bindings = HashMap::new();
        bindings.insert(a.account.clone(), seed_bound(17));
        bindings.insert(b.account.clone(), seed_bound(23));

        let review = build(vec![a, b], &bindings, &HashMap::new());

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

        let first = build(
            vec![a.clone(), b.clone(), orphan.clone()],
            &bindings,
            &HashMap::new(),
        );
        let again = build(vec![orphan, b, a], &bindings, &HashMap::new());

        assert_eq!(
            first.items, again.items,
            "input order must not change what the operator sees"
        );
    }

    #[test]
    fn closed_accounts_leave_the_queue() {
        let mut closed = observed("github", "gh-9", None);
        closed.is_closed = true;

        let review = build(vec![closed], &HashMap::new(), &HashMap::new());

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
                provenance: Provenance::Resolved,
            },
        );

        let review = build(vec![bot], &bindings, &HashMap::new());

        assert!(review.items.is_empty());
        assert_eq!(review.rates.excluded, 1);
        assert_eq!(review.rates.bound, 0, "excluded is its own state");
    }
}
