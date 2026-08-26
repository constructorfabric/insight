use uuid::Uuid;

use super::resolution::BindingRow;
use super::seed::SourceAccountKey;
use crate::infra::identity_evidence::ObservedAccount;

/// Column widths a binding lands in (`001_persons.sql`). 320 is also the limit
/// `POST /v1/profiles` states for an id value, so one number is one contract
/// across both entrances.
pub const MAX_VALUE_ID_CHARS: usize = 320;
pub const MAX_SOURCE_TYPE_CHARS: usize = 100;

/// The journal reason marking a row the login bootstrap wrote.
pub const LOGIN_BOOTSTRAP_REASON: &str = "login-bootstrap";

/// Namespace for [`derived_person_id`]. Fixed, and used for nothing else.
const PERSON_NAMESPACE: Uuid = Uuid::from_u128(0x9f2c_6ad1_4e83_4f27_bd51_7c0a_38e9_1b64);

/// Why a principal may not be provisioned. Each variant is an answer about the
/// principal, not a fault — the caller turns them into refusals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A field is unusable: empty, or longer than the column it lands in.
    Invalid {
        field: &'static str,
        message: String,
    },
    /// The token asserts a tenant this journal is not keyed by, or the service
    /// cannot say which tenant that is.
    Tenant(TenantRefusal),
    /// The source has deactivated the account.
    Closed,
    /// The account carries an address, so identity resolution links it.
    Addressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantRefusal {
    /// The service has no configured tenant, so it cannot know which journal
    /// it would be writing into.
    Unconfigured,
    /// The asserted tenant is not the configured one.
    Mismatch,
}

/// What the caller asked for, already parsed.
#[derive(Debug, Clone, Copy)]
pub struct Principal<'a> {
    pub source_type: &'a str,
    pub external_id: &'a str,
    pub asserted_tenant: Uuid,
}

/// Why an address cannot be used to resolve a login at all — decided before any
/// query runs, so a shape that could never answer never reaches the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RosterEmailRefusal {
    /// The caller resolved no address. In email mode that means a token reached
    /// the authenticator without the claim it was configured to resolve by.
    AddressMissing,
    /// No roster is declared, so there is no source whose addresses may admit
    /// anyone. Widening to every source instead is the one thing this lookup
    /// may not do.
    RosterUnconfigured,
    /// The caller's token carries no tenant. An address is not unique across
    /// tenants sharing a journal, so without one the lookup cannot be confined.
    TenantUnresolved,
}

/// An address the roster may be asked about, with the two things that confine
/// the question.
#[derive(Debug, PartialEq, Eq)]
pub struct RosterEmail<'a> {
    pub tenant_id: Uuid,
    pub source_type: &'a str,
    pub address: &'a str,
}

/// Whether this address may be asked about at all.
///
/// # Errors
///
/// Returns the refusal describing which confinement is missing.
pub fn parse_roster_email<'a>(
    address: &'a str,
    roster_source_type: &'a str,
    tenant_id: Uuid,
) -> Result<RosterEmail<'a>, RosterEmailRefusal> {
    let address = address.trim();
    if address.is_empty() {
        return Err(RosterEmailRefusal::AddressMissing);
    }
    let source_type = roster_source_type.trim();
    if source_type.is_empty() {
        return Err(RosterEmailRefusal::RosterUnconfigured);
    }
    if tenant_id.is_nil() {
        return Err(RosterEmailRefusal::TenantUnresolved);
    }
    Ok(RosterEmail {
        tenant_id,
        source_type,
        address,
    })
}

/// The person a login resolves to, and how many the roster offered.
#[derive(Debug, PartialEq, Eq)]
pub struct RosterEmailMatch {
    pub person_id: Uuid,
    /// More than one means the roster states this address for several people.
    /// The seed refuses to auto-link that shape and an operator may have split
    /// them deliberately, so answering it is a decision the caller records
    /// rather than one the journal makes quietly.
    pub candidates: usize,
}

/// Pick the person an address resolves to. `None` when the roster states it for
/// nobody who still holds a live account under it.
#[must_use]
pub fn choose_roster_email_match(candidates: &[Uuid]) -> Option<RosterEmailMatch> {
    candidates.first().map(|&person_id| RosterEmailMatch {
        person_id,
        candidates: candidates.len(),
    })
}

/// Trim and bound the principal a request names.
///
/// Left unbounded, an over-long id is a database error under strict SQL and,
/// under a lax mode, a silently truncated row that neither the write guard nor
/// the read-back can match again — so every attempt appends another unusable
/// row and still refuses the caller.
pub fn parse_principal<'a>(
    source_type: &'a str,
    external_id: &'a str,
    asserted_tenant: Uuid,
) -> Result<Principal<'a>, Refusal> {
    let source_type = source_type.trim();
    let external_id = external_id.trim();

    let too_long = |field: &'static str, limit: usize| Refusal::Invalid {
        field,
        message: format!("{field} must be at most {limit} characters"),
    };
    let required = |field: &'static str| Refusal::Invalid {
        field,
        message: format!("{field} must not be empty"),
    };

    if source_type.is_empty() {
        return Err(required("source_type"));
    }
    if external_id.is_empty() {
        return Err(required("external_id"));
    }
    if external_id.chars().count() > MAX_VALUE_ID_CHARS {
        return Err(too_long("external_id", MAX_VALUE_ID_CHARS));
    }
    if source_type.chars().count() > MAX_SOURCE_TYPE_CHARS {
        return Err(too_long("source_type", MAX_SOURCE_TYPE_CHARS));
    }
    if asserted_tenant.is_nil() {
        return Err(required("tenant_id"));
    }

    Ok(Principal {
        source_type,
        external_id,
        asserted_tenant,
    })
}

/// The tenant a provisioned binding may be written under.
///
/// A row under any other tenant is invisible to the persons-seed: never
/// adopted, and a second person minted for the same account on its next run.
pub fn provisioning_tenant(configured: &str, asserted: Uuid) -> Result<Uuid, TenantRefusal> {
    let configured = configured.trim();
    if configured.is_empty() {
        return Err(TenantRefusal::Unconfigured);
    }
    let Ok(configured) = Uuid::parse_str(configured) else {
        return Err(TenantRefusal::Unconfigured);
    };
    if configured != asserted {
        return Err(TenantRefusal::Mismatch);
    }
    Ok(configured)
}

/// The person a given account provisions to.
///
/// Derived, not random: the journal's natural key carries `person_id`, so two
/// concurrent logins minting random ids would both insert and split one human
/// across two people. Deriving it makes the racers agree.
pub fn derived_person_id(tenant: Uuid, account: &SourceAccountKey) -> Uuid {
    // The unit separator is what keeps the parts from running together: plain
    // concatenation lets ("a", "bc") and ("ab", "c") name one person.
    let name = format!(
        "{tenant}\u{1f}{}\u{1f}{}\u{1f}{}",
        account.source_type, account.source_id, account.account_id
    );
    Uuid::new_v5(&PERSON_NAMESPACE, name.as_bytes())
}

/// The binding to append for this principal, or why it may not be provisioned.
///
/// Every rule that decides WHO may be minted lives here, over values, so it is
/// answerable without a database or an evidence client.
pub fn decide(
    principal: Principal<'_>,
    observed: &ObservedAccount,
    tenant: Uuid,
    now: sea_orm::prelude::DateTime,
) -> Result<BindingRow, Refusal> {
    // Gone from its source: the review queue drops such accounts before it
    // counts anything, and entering through one would keep a door the roster
    // has already shut.
    if observed.is_closed {
        return Err(Refusal::Closed);
    }

    // An account with an address is the BATCH's to resolve, and minting here
    // would do harm rather than duplicate work: the seed groups by address, so
    // it would have attached this account to whoever already holds that one. A
    // person minted first takes the binding, and the seed then reads the group
    // as a conflict between two persons with no operator decision to settle it
    // — it keeps both, so one human stays split until somebody merges by hand.
    //
    // The deferral is to the batch OR to the operator, not to the batch alone:
    // where the account's source states no id, no seed run can bind it either
    // (`seed::resolve_assignments`), and it waits on the review queue as
    // `no_source_id` instead. Refusing here is still right — a login may not
    // decide who a claimed address belongs to — but nothing about the refusal
    // promises automation will.
    if observed.email.is_some() {
        return Err(Refusal::Addressed);
    }

    let account = SourceAccountKey {
        source_type: principal.source_type.to_owned(),
        // The instance the evidence names, never one of our choosing: the
        // persons-seed matches accounts on the whole triple, so a binding
        // under any other id would never be recognised as this account's.
        source_id: observed.source_id,
        account_id: principal.external_id.to_owned(),
    };

    Ok(BindingRow {
        person_id: derived_person_id(tenant, &account),
        account,
        // Automation, not an operator decision: an operator-authored binding
        // settles a contested group (ADR-0003), and this one settles nothing.
        author_person_id: Uuid::nil(),
        reason: LOGIN_BOOTSTRAP_REASON.to_owned(),
        created_at: now,
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    type R = Result<(), Box<dyn Error>>;

    const TENANT: Uuid = Uuid::from_u128(9);

    fn account(account_id: &str) -> SourceAccountKey {
        SourceAccountKey {
            source_type: "github".to_owned(),
            source_id: Uuid::from_u128(0xaa01),
            account_id: account_id.to_owned(),
        }
    }

    fn observed(email: Option<&str>, is_closed: bool) -> ObservedAccount {
        ObservedAccount {
            source_id: Uuid::from_u128(0xaa01),
            is_closed,
            email: email.map(str::to_owned),
        }
    }

    fn now() -> sea_orm::prelude::DateTime {
        // A fixed instant: nothing here depends on the clock, and a literal
        // keeps the rows a test builds comparable.
        chrono::DateTime::UNIX_EPOCH.naive_utc()
    }

    #[test]
    fn a_principal_is_bounded_by_the_columns_it_lands_in() {
        let long_id = "x".repeat(MAX_VALUE_ID_CHARS + 1);
        let long_source = "s".repeat(MAX_SOURCE_TYPE_CHARS + 1);
        for (case, source_type, external_id, tenant, field) in [
            ("empty source_type", "  ", "octocat", TENANT, "source_type"),
            ("empty external_id", "github", " ", TENANT, "external_id"),
            (
                "over-long external_id",
                "github",
                long_id.as_str(),
                TENANT,
                "external_id",
            ),
            (
                "over-long source_type",
                long_source.as_str(),
                "octocat",
                TENANT,
                "source_type",
            ),
            ("nil tenant", "github", "octocat", Uuid::nil(), "tenant_id"),
        ] {
            let refused = parse_principal(source_type, external_id, tenant);
            match refused {
                Err(Refusal::Invalid { field: got, .. }) => {
                    assert_eq!(got, field, "wrong field named for: {case}");
                }
                other => panic!("should refuse {case}, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_principal_is_trimmed_not_merely_accepted() -> R {
        let principal = parse_principal("  github  ", "  octocat  ", TENANT)
            .map_err(|r| format!("a well-formed principal was refused: {r:?}"))?;

        assert_eq!(principal.source_type, "github");
        assert_eq!(principal.external_id, "octocat");
        Ok(())
    }

    #[test]
    fn a_write_goes_only_to_the_journals_own_tenant() {
        for (case, configured, asserted, expected) in [
            ("matching", TENANT.to_string(), TENANT, Ok(TENANT)),
            (
                "another tenant",
                TENANT.to_string(),
                Uuid::from_u128(10),
                Err(TenantRefusal::Mismatch),
            ),
            (
                "unconfigured",
                String::new(),
                TENANT,
                Err(TenantRefusal::Unconfigured),
            ),
            (
                "unreadable configuration",
                "not-a-uuid".to_owned(),
                TENANT,
                Err(TenantRefusal::Unconfigured),
            ),
        ] {
            assert_eq!(
                provisioning_tenant(&configured, asserted),
                expected,
                "wrong answer for: {case}"
            );
        }
    }

    #[test]
    fn only_an_account_the_batch_cannot_resolve_is_provisioned() -> R {
        let principal = parse_principal("github", "octocat", TENANT)
            .map_err(|r| format!("a well-formed principal was refused: {r:?}"))?;

        for (case, evidence, expected) in [
            (
                "closed at its source",
                observed(None, true),
                Some(Refusal::Closed),
            ),
            (
                "carries an address",
                observed(Some("jane@example.com"), false),
                Some(Refusal::Addressed),
            ),
            (
                "closed AND addressed — closure answers first",
                observed(Some("jane@example.com"), true),
                Some(Refusal::Closed),
            ),
            ("no address, still open", observed(None, false), None),
        ] {
            let decided = decide(principal, &evidence, TENANT, now());
            match (decided, expected) {
                (Err(got), Some(want)) => assert_eq!(got, want, "wrong refusal for: {case}"),
                (Ok(row), None) => {
                    assert_eq!(row.reason, LOGIN_BOOTSTRAP_REASON, "case: {case}");
                    assert!(
                        row.author_person_id.is_nil(),
                        "case: {case} — not an operator"
                    );
                }
                (got, want) => panic!("case {case}: got {got:?}, wanted {want:?}"),
            }
        }
        Ok(())
    }

    #[test]
    fn the_binding_carries_the_instance_the_evidence_named() -> R {
        let principal = parse_principal("github", "octocat", TENANT)
            .map_err(|r| format!("a well-formed principal was refused: {r:?}"))?;
        let evidence = ObservedAccount {
            source_id: Uuid::from_u128(0xbb02),
            is_closed: false,
            email: None,
        };

        let row = decide(principal, &evidence, TENANT, now())
            .map_err(|r| format!("should provision, refused: {r:?}"))?;

        // Not an id of our choosing: the persons-seed recognises an account by
        // the whole triple, so anything else is never adopted.
        assert_eq!(row.account.source_id, Uuid::from_u128(0xbb02));
        Ok(())
    }

    #[test]
    fn the_same_account_always_derives_the_same_person() {
        assert_eq!(
            derived_person_id(TENANT, &account("octocat")),
            derived_person_id(TENANT, &account("octocat")),
        );
        assert!(!derived_person_id(TENANT, &account("octocat")).is_nil());
    }

    #[test]
    fn distinct_accounts_never_derive_one_person() {
        let mut seen = std::collections::HashSet::new();
        for (case, tenant, account) in [
            ("baseline", TENANT, account("octocat")),
            ("another account", TENANT, account("octocat2")),
            ("another tenant", Uuid::from_u128(10), account("octocat")),
            // The pair plain concatenation would collide.
            ("separator on the left", TENANT, account("a\u{1f}bc")),
            ("separator on the right", TENANT, account("ab\u{1f}c")),
            (
                "another source",
                TENANT,
                SourceAccountKey {
                    source_type: "gitlab".to_owned(),
                    source_id: Uuid::from_u128(0xaa01),
                    account_id: "octocat".to_owned(),
                },
            ),
        ] {
            assert!(
                seen.insert(derived_person_id(tenant, &account)),
                "collided with an earlier case at: {case}"
            );
        }
    }

    #[test]
    fn an_address_is_asked_about_only_when_every_confinement_is_present() -> R {
        let tenant = Uuid::from_u128(7);

        let asked = parse_roster_email("  ivan@vz.com  ", " bamboohr ", tenant)
            .map_err(|r| format!("a well-formed address was refused: {r:?}"))?;
        assert_eq!(asked.address, "ivan@vz.com", "trimmed, never re-cased");
        assert_eq!(asked.source_type, "bamboohr");
        assert_eq!(asked.tenant_id, tenant);

        // Each missing confinement refuses on its own, and says which — the
        // three mean different operator mistakes and would otherwise all read
        // as "this person cannot log in".
        for (address, source, tenant_id, expected) in [
            (
                "   ",
                "bamboohr",
                tenant,
                RosterEmailRefusal::AddressMissing,
            ),
            (
                "ivan@vz.com",
                "  ",
                tenant,
                RosterEmailRefusal::RosterUnconfigured,
            ),
            (
                "ivan@vz.com",
                "bamboohr",
                Uuid::nil(),
                RosterEmailRefusal::TenantUnresolved,
            ),
        ] {
            assert!(
                matches!(
                    parse_roster_email(address, source, tenant_id),
                    Err(refusal) if refusal == expected
                ),
                "{address:?}/{source:?} must be refused as {expected:?}",
            );
        }
        Ok(())
    }

    #[test]
    fn a_contested_address_still_resolves_but_says_it_was_contested() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);

        assert_eq!(choose_roster_email_match(&[]), None);
        assert_eq!(
            choose_roster_email_match(&[first]),
            Some(RosterEmailMatch {
                person_id: first,
                candidates: 1
            })
        );
        // The install answers rather than refusing, so the count is what the
        // caller records. Losing it here would make the choice unauditable.
        assert_eq!(
            choose_roster_email_match(&[first, second]),
            Some(RosterEmailMatch {
                person_id: first,
                candidates: 2
            })
        );
    }
}
