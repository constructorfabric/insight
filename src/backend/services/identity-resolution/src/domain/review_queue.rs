use std::collections::HashMap;

use uuid::Uuid;

use super::provenance::Provenance;
use super::resolution::EXCLUDED_PERSON;
use super::seed::{KnownBinding, SourceAccountKey, normalize_email};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Contested,
    BindingConflict,
    ProvisionedAtLogin,
    MintedFromRoster,
    NoSourceId,
    NoEvidence,
}

impl ItemKind {
    pub const ALL: [Self; 6] = [
        Self::Contested,
        Self::BindingConflict,
        Self::ProvisionedAtLogin,
        Self::MintedFromRoster,
        Self::NoSourceId,
        Self::NoEvidence,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Contested => "contested",
            Self::BindingConflict => "binding_conflict",
            Self::ProvisionedAtLogin => "provisioned_at_login",
            Self::MintedFromRoster => "minted_from_roster",
            Self::NoSourceId => "no_source_id",
            Self::NoEvidence => "no_evidence",
        }
    }

    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.label() == label)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountDescription {
    pub display_name: Option<String>,
    pub job_title: Option<String>,
    pub department: Option<String>,
    pub status: Option<String>,
    pub manager_email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub kind: ItemKind,
    pub account: SourceAccountKey,
    pub email: Option<String>,
    pub username: Option<String>,
    pub description: AccountDescription,
    pub bound_to: Option<Uuid>,
    pub candidates: Vec<Uuid>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResolutionRates {
    pub observed: usize,
    pub bound: usize,
    pub pending: usize,
    pub no_source_id: usize,
    pub no_evidence: usize,
    pub excluded: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Review {
    pub items: Vec<QueueItem>,
    pub rates: ResolutionRates,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceAccount {
    pub account: SourceAccountKey,
    pub email: Option<String>,
    pub username: Option<String>,
    pub description: AccountDescription,
    pub is_closed: bool,
    pub states_binding_id: bool,
}

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
                    rates.no_source_id += 1;
                    items.push(item(ItemKind::NoSourceId, account, None, candidates));
                } else {
                    rates.pending += 1;
                }
            }
        }
    }

    items.extend(binding_conflicts(&by_email, bindings));

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

#[derive(Debug, PartialEq, Eq)]
pub enum KindFilter {
    Any,
    Only(Vec<ItemKind>),
}

impl KindFilter {
    #[must_use]
    pub fn from_labels(labels: &[String]) -> Self {
        if labels.is_empty() {
            return Self::Any;
        }

        Self::Only(
            labels
                .iter()
                .filter_map(|label| ItemKind::from_label(label))
                .collect(),
        )
    }

    fn admits(&self, kind: ItemKind) -> bool {
        match self {
            Self::Any => true,
            Self::Only(kinds) => kinds.contains(&kind),
        }
    }
}

#[must_use]
pub fn narrow(items: Vec<QueueItem>, sources: &[String], kinds: &KindFilter) -> Vec<QueueItem> {
    items
        .into_iter()
        .filter(|item| {
            (sources.is_empty() || sources.contains(&item.account.source_type))
                && kinds.admits(item.kind)
        })
        .collect()
}

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
            continue;
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

fn has_matchable_evidence(account: &EvidenceAccount) -> bool {
    account.email.is_some()
}

fn address_is_claimed(account: &EvidenceAccount, claimed: &HashMap<String, Uuid>) -> bool {
    address_holder(account.email.as_deref(), None, claimed).is_some()
}

#[must_use]
pub fn address_holder(
    email: Option<&str>,
    binding: Option<&KnownBinding>,
    claimed: &HashMap<String, Uuid>,
) -> Option<Uuid> {
    if binding.is_some() {
        return None;
    }

    email
        .map(normalize_email)
        .and_then(|email| claimed.get(&email).copied())
        .filter(|person| *person != EXCLUDED_PERSON)
}

fn unconfirmed_item(account: &EvidenceAccount, binding: &KnownBinding) -> Option<QueueItem> {
    if binding.is_operator_authored() {
        return None;
    }

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
        let contested_a = observed("github", "gh-1", Some("shared@example.com"));
        let contested_b = observed("jira", "jr-1", Some("shared@example.com"));
        let newcomer = observed("slack", "slk-1", Some("shared@example.com"));
        let login = observed("github", "gh-9", None);
        let roster = observed("bamboohr", "e-1", None);
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
        let claim = without_source_id("github-commit-email", "bot@corp.com", Some("bot@corp.com"));
        let mut claimed = HashMap::new();
        claimed.insert("bot@corp.com".to_owned(), EXCLUDED_PERSON);

        let review = build(vec![claim], &HashMap::new(), &claimed);

        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].kind, ItemKind::NoSourceId);
    }

    #[test]
    fn a_contested_address_outranks_a_missing_source_id() {
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

    fn queued(kind: ItemKind, source: &str, account_id: &str) -> QueueItem {
        QueueItem {
            kind,
            account: SourceAccountKey {
                source_type: source.to_owned(),
                source_id: Uuid::from_u128(1),
                account_id: account_id.to_owned(),
            },
            email: None,
            username: None,
            description: AccountDescription::default(),
            bound_to: None,
            candidates: Vec::new(),
        }
    }

    fn ids(items: &[QueueItem]) -> Vec<&str> {
        items
            .iter()
            .map(|i| i.account.account_id.as_str())
            .collect()
    }

    fn labels(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|s| (*s).to_owned()).collect()
    }

    fn sample_queue() -> Vec<QueueItem> {
        vec![
            queued(ItemKind::BindingConflict, "gitlab", "x-1"),
            queued(ItemKind::Contested, "gitlab", "c-1"),
            queued(ItemKind::NoEvidence, "gitlab", "e-1"),
            queued(ItemKind::NoEvidence, "github", "e-2"),
            queued(ItemKind::NoSourceId, "hr", "n-1"),
        ]
    }

    #[test]
    fn narrowing_keeps_queue_order_and_applies_both_filters() {
        for (case, sources, kinds, expected) in [
            (
                "no filter keeps everything",
                labels(&[]),
                KindFilter::Any,
                vec!["x-1", "c-1", "e-1", "e-2", "n-1"],
            ),
            (
                "sources only",
                labels(&["gitlab", "hr"]),
                KindFilter::Any,
                vec!["x-1", "c-1", "e-1", "n-1"],
            ),
            (
                "a known kind keeps only its items",
                labels(&[]),
                KindFilter::from_labels(&labels(&["no_evidence"])),
                vec!["e-1", "e-2"],
            ),
            (
                "several kinds",
                labels(&[]),
                KindFilter::from_labels(&labels(&["contested", "no_source_id"])),
                vec!["c-1", "n-1"],
            ),
            (
                "unknown kinds only match nothing",
                labels(&[]),
                KindFilter::from_labels(&labels(&["nope"])),
                vec![],
            ),
            (
                "an unknown kind beside a known one keeps the known one",
                labels(&[]),
                KindFilter::from_labels(&labels(&["nope", "contested"])),
                vec!["c-1"],
            ),
            (
                "sources and kinds together",
                labels(&["gitlab"]),
                KindFilter::from_labels(&labels(&["no_evidence"])),
                vec!["e-1"],
            ),
            (
                "an unknown source matches nothing",
                labels(&["nope"]),
                KindFilter::Any,
                vec![],
            ),
        ] {
            let kept = narrow(sample_queue(), &sources, &kinds);

            assert_eq!(ids(&kept), expected, "wrong items for: {case}");
        }
    }

    #[test]
    fn no_kind_labels_is_no_filter_while_unknown_labels_is_an_empty_one() {
        assert_eq!(KindFilter::from_labels(&[]), KindFilter::Any);
        assert_eq!(
            KindFilter::from_labels(&labels(&["nope"])),
            KindFilter::Only(Vec::new())
        );
    }

    #[test]
    fn every_kind_label_round_trips() {
        for kind in ItemKind::ALL {
            assert_eq!(ItemKind::from_label(kind.label()), Some(kind));
        }
        assert_eq!(ItemKind::from_label("nope"), None);
        assert_eq!(ItemKind::from_label("Contested"), None);
    }

    #[test]
    fn every_queue_kind_has_the_wire_label_its_consumers_switch_on() {
        for (kind, expected) in [
            (ItemKind::Contested, "contested"),
            (ItemKind::BindingConflict, "binding_conflict"),
            (ItemKind::ProvisionedAtLogin, "provisioned_at_login"),
            (ItemKind::MintedFromRoster, "minted_from_roster"),
            (ItemKind::NoSourceId, "no_source_id"),
            (ItemKind::NoEvidence, "no_evidence"),
        ] {
            assert_eq!(kind.label(), expected, "wrong label for {kind:?}");
        }
        assert_eq!(ItemKind::ALL.len(), 6);
    }

    #[test]
    fn the_address_holder_is_the_person_an_unbound_address_resolves_to() {
        let holder = Uuid::from_u128(7);
        let claimed = HashMap::from([
            ("a@example.com".to_owned(), holder),
            ("bot@example.com".to_owned(), EXCLUDED_PERSON),
        ]);
        let bound = seed_bound(9);
        let excluded = KnownBinding {
            person_id: EXCLUDED_PERSON,
            author_person_id: Uuid::nil(),
            provenance: Provenance::Resolved,
        };

        for (case, email, binding, expected) in [
            ("bound", Some("a@example.com"), Some(&bound), None),
            (
                "bound to the excluded person",
                Some("a@example.com"),
                Some(&excluded),
                None,
            ),
            (
                "unbound and claimed",
                Some("a@example.com"),
                None,
                Some(holder),
            ),
            (
                "claimed by the excluded person",
                Some("bot@example.com"),
                None,
                None,
            ),
            ("no address", None, None, None),
            (
                "address differing only in case",
                Some("A@Example.COM"),
                None,
                Some(holder),
            ),
            ("unclaimed address", Some("b@example.com"), None, None),
        ] {
            assert_eq!(
                address_holder(email, binding, &claimed),
                expected,
                "wrong holder for: {case}"
            );
        }
    }
}
