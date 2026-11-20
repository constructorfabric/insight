use uuid::Uuid;

use super::resolution::BindingRow;
use super::seed::SourceAccountKey;
use crate::infra::identity_evidence::ObservedAccount;

pub const MAX_VALUE_ID_CHARS: usize = 320;
pub const MAX_SOURCE_TYPE_CHARS: usize = 30;

pub const LOGIN_BOOTSTRAP_REASON: &str = "login-bootstrap";

const PERSON_NAMESPACE: Uuid = Uuid::from_u128(0x9f2c_6ad1_4e83_4f27_bd51_7c0a_38e9_1b64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Invalid {
        field: &'static str,
        message: String,
    },
    Tenant(TenantRefusal),
    Closed,
    Addressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantRefusal {
    Unconfigured,
    Mismatch,
}

#[derive(Debug, Clone, Copy)]
pub struct Principal<'a> {
    pub source_type: &'a str,
    pub external_id: &'a str,
    pub asserted_tenant: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RosterEmailRefusal {
    AddressMissing,
    RosterUnconfigured,
    TenantUnresolved,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RosterEmail<'a> {
    pub tenant_id: Uuid,
    pub source_type: &'a str,
    pub address: &'a str,
}

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

#[derive(Debug, PartialEq, Eq)]
pub struct RosterEmailMatch {
    pub person_id: Uuid,
    pub candidates: usize,
}

#[must_use]
pub fn choose_roster_email_match(candidates: &[Uuid]) -> Option<RosterEmailMatch> {
    candidates.first().map(|&person_id| RosterEmailMatch {
        person_id,
        candidates: candidates.len(),
    })
}

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

pub fn derived_person_id(tenant: Uuid, account: &SourceAccountKey) -> Uuid {
    let name = format!(
        "{tenant}\u{1f}{}\u{1f}{}\u{1f}{}",
        account.source_type, account.source_id, account.account_id
    );
    Uuid::new_v5(&PERSON_NAMESPACE, name.as_bytes())
}

pub fn decide(
    principal: Principal<'_>,
    observed: &ObservedAccount,
    tenant: Uuid,
    now: sea_orm::prelude::DateTime,
) -> Result<BindingRow, Refusal> {
    if observed.is_closed {
        return Err(Refusal::Closed);
    }

    if observed.email.is_some() {
        return Err(Refusal::Addressed);
    }

    let account = SourceAccountKey {
        source_type: principal.source_type.to_owned(),
        source_id: observed.source_id,
        account_id: principal.external_id.to_owned(),
    };

    Ok(BindingRow {
        person_id: derived_person_id(tenant, &account),
        account,
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
    fn a_principal_at_the_column_width_is_accepted() -> R {
        let source_type = "s".repeat(MAX_SOURCE_TYPE_CHARS);
        let external_id = "x".repeat(MAX_VALUE_ID_CHARS);
        parse_principal(&source_type, &external_id, TENANT)
            .map_err(|r| format!("a principal that fits its columns was refused: {r:?}"))?;
        Ok(())
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
        assert_eq!(
            choose_roster_email_match(&[first, second]),
            Some(RosterEmailMatch {
                person_id: first,
                candidates: 2
            })
        );
    }
}
