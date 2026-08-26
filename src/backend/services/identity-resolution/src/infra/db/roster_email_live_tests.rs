//! The login-bootstrap resolve by address, against a live `MariaDB`.
//!
//! Three of the four confinements this query relies on are the database's
//! answer, not Rust's — the tenant filter, the collation that decides whether
//! two spellings are one address, and the live-binding requirement that makes
//! an operator's exclusion bite. None can be established by a unit test, so
//! these cases run the real query against the real schema.
//!
//! Addresses carry the fixture's tenant in their local part. These are the
//! repo's first tenant-agnostic-key live tests: the fixture never cleans up and
//! the README points `INTEGRATION_TESTS_MARIADB_URL` at a shared MariaDB, so a
//! fixed address would make two concurrent runs race.

use super::persons_repo;
use super::test_fixture::{SOURCE_TYPE, fixture_or_skip};
use crate::domain::resolution::EXCLUDED_PERSON;

type TestResult = anyhow::Result<()>;

/// A source that states addresses without being trusted to say who exists.
const CHAT: &str = "zulip-proxy";

#[tokio::test]
async fn an_address_resolves_whatever_case_the_token_carries_it_in() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let addr = format!("ivan.petrov.{}@vz.com", f.tenant.simple());
    let person = f.account_holder(&addr).await?;

    // An HR roster may state an address in any case, and an IdP hands over
    // whatever its directory holds. Neither side folds anything: `value_id` is
    // `utf8mb4_unicode_ci` since migration 004.
    for typed in [addr.clone(), addr.to_uppercase(), format!("  {addr}  ")] {
        assert_eq!(
            persons_repo::resolve_person_ids_by_roster_email(&f.db, f.tenant, SOURCE_TYPE, &typed)
                .await?,
            vec![person],
            "{typed:?} must reach the same person",
        );
    }
    Ok(())
}

#[tokio::test]
async fn only_the_roster_s_own_addresses_admit_anyone() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // One person, two addresses: the roster states one, their chat account the
    // other. Both are `value_type='email'` rows under the same tenant and the
    // same source instance — only the source TYPE tells them apart.
    let roster_addr = format!("ivan.{}@vz.com", f.tenant.simple());
    let chat_addr = format!("ivan.{}@chat.example", f.tenant.simple());
    let person = f.account_holder(&roster_addr).await?;
    f.observed_from(CHAT, person, "email", &chat_addr).await?;

    assert_eq!(
        persons_repo::resolve_person_ids_by_roster_email(
            &f.db,
            f.tenant,
            SOURCE_TYPE,
            &roster_addr
        )
        .await?,
        vec![person],
    );
    assert!(
        persons_repo::resolve_person_ids_by_roster_email(&f.db, f.tenant, SOURCE_TYPE, &chat_addr)
            .await?
            .is_empty(),
        "an address only the chat ever stated must not resolve a login",
    );
    Ok(())
}

#[tokio::test]
async fn an_address_stated_under_another_value_type_admits_nobody() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // Rosters commonly publish the UPN as a username, and a UPN looks exactly
    // like an address. Only an `email` observation may admit.
    let addr = format!("upn.{}@vz.com", f.tenant.simple());
    let person = f
        .account_holder(&format!("real.{}@vz.com", f.tenant.simple()))
        .await?;
    f.observed(person, "username", &addr).await?;

    assert!(
        persons_repo::resolve_person_ids_by_roster_email(&f.db, f.tenant, SOURCE_TYPE, &addr)
            .await?
            .is_empty(),
    );
    Ok(())
}

#[tokio::test]
async fn an_excluded_account_no_longer_admits_anyone() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // Exclusion (ADR-0003) is recorded against the ACCOUNT, as a newer `id` row
    // naming the sentinel. The address row written before it is never
    // superseded — the seed stops emitting for an excluded account — so a query
    // that read addresses alone would keep admitting the person it named.
    let addr = format!("svc-deploy.{}@vz.com", f.tenant.simple());
    let account = format!("acct-svc-{}", f.tenant.simple());
    let person = f.person(&addr).await?;
    f.bound_at(&account, person, "auto-seed-link", 60).await?;

    assert_eq!(
        persons_repo::resolve_person_ids_by_roster_email(&f.db, f.tenant, SOURCE_TYPE, &addr)
            .await?,
        vec![person],
        "precondition: a live roster account resolves",
    );

    f.bound_at(&account, EXCLUDED_PERSON, "excluded", 0).await?;

    assert!(
        persons_repo::resolve_person_ids_by_roster_email(&f.db, f.tenant, SOURCE_TYPE, &addr)
            .await?
            .is_empty(),
        "an excluded account must not sign in, address row or not",
    );
    Ok(())
}

#[tokio::test]
async fn an_address_with_no_account_behind_it_admits_nobody() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // An address observation nobody holds an account for is not evidence that a
    // human may enter — the roster's `id` binding is.
    let addr = format!("orphan.{}@vz.com", f.tenant.simple());
    f.person(&addr).await?;

    assert!(
        persons_repo::resolve_person_ids_by_roster_email(&f.db, f.tenant, SOURCE_TYPE, &addr)
            .await?
            .is_empty(),
    );
    Ok(())
}

#[tokio::test]
async fn another_tenant_s_roster_cannot_resolve_this_tenant_s_login() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // The shape an address key cannot survive without the tenant filter: a
    // neighbour on the same database states the same address in its own roster,
    // more recently. Ours must win, and theirs must be invisible.
    let addr = format!("shared.{}@vz.com", f.tenant.simple());
    let ours = f.account_holder(&addr).await?;
    let neighbour = f.in_another_tenant();
    let theirs = neighbour.account_holder(&addr).await?;

    assert_eq!(
        persons_repo::resolve_person_ids_by_roster_email(&f.db, f.tenant, SOURCE_TYPE, &addr)
            .await?,
        vec![ours],
        "our tenant resolves our person, never the neighbour's newer row",
    );
    assert_eq!(
        persons_repo::resolve_person_ids_by_roster_email(
            &f.db,
            neighbour.tenant,
            SOURCE_TYPE,
            &addr
        )
        .await?,
        vec![theirs],
    );
    Ok(())
}

#[tokio::test]
async fn two_persons_on_one_roster_address_are_both_returned() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // The seed refuses to auto-link a contested address, and an operator may
    // have split two people on purpose. The query hands both back so the caller
    // can say so; silently returning one would overrule that decision at the
    // door with nothing to read afterwards.
    let addr = format!("contested.{}@vz.com", f.tenant.simple());
    let first = f.account_holder(&addr).await?;
    let second = f.account_holder(&addr).await?;

    let candidates =
        persons_repo::resolve_person_ids_by_roster_email(&f.db, f.tenant, SOURCE_TYPE, &addr)
            .await?;

    assert_eq!(candidates.len(), 2, "both persons must be reported");
    assert!(candidates.contains(&first) && candidates.contains(&second));
    Ok(())
}

#[tokio::test]
async fn an_address_the_roster_has_replaced_no_longer_admits() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // `persons` is append-only, so a person keeps every address they were ever
    // observed under. Only the newest counts: an alias the roster has moved on
    // from is not evidence about who is signing in.
    let old_addr = format!("ivan.old.{}@vz.com", f.tenant.simple());
    let new_addr = format!("ivan.new.{}@vz.com", f.tenant.simple());
    let person = f.account_holder(&old_addr).await?;
    f.observed_at(person, "email", &old_addr, 86_400).await?;
    f.observed_at(person, "email", &new_addr, 0).await?;

    assert_eq!(
        persons_repo::resolve_person_ids_by_roster_email(&f.db, f.tenant, SOURCE_TYPE, &new_addr)
            .await?,
        vec![person],
        "the address the roster states now resolves",
    );
    assert!(
        persons_repo::resolve_person_ids_by_roster_email(&f.db, f.tenant, SOURCE_TYPE, &old_addr)
            .await?
            .is_empty(),
        "the address it has replaced does not",
    );
    Ok(())
}

#[tokio::test]
async fn a_reassigned_alias_admits_its_new_owner_and_not_the_leaver() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // The reason the currency rule matters. A leaver's alias is handed to a new
    // hire: both roster accounts have stated it, and matching any observation
    // would sign the new hire in as the leaver — whose org position, history and
    // roles they would inherit.
    let alias = format!("sales.{}@vz.com", f.tenant.simple());
    // The leaver holds a live roster account whose CURRENT address is their own
    // — the alias is something they were observed under earlier.
    let leaver = f
        .account_holder(&format!("leaver.{}@vz.com", f.tenant.simple()))
        .await?;
    f.observed_at(leaver, "email", &alias, 172_800).await?;
    // The new hire's roster account states the alias now.
    let newcomer = f.account_holder(&alias).await?;

    assert_eq!(
        persons_repo::resolve_person_ids_by_roster_email(&f.db, f.tenant, SOURCE_TYPE, &alias)
            .await?,
        vec![newcomer],
        "only whoever the roster states the alias for NOW",
    );
    Ok(())
}

#[tokio::test]
async fn an_address_nobody_stated_resolves_nobody() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    f.account_holder(&format!("someone.{}@vz.com", f.tenant.simple()))
        .await?;

    assert!(
        persons_repo::resolve_person_ids_by_roster_email(
            &f.db,
            f.tenant,
            SOURCE_TYPE,
            &format!("stranger.{}@vz.com", f.tenant.simple()),
        )
        .await?
        .is_empty(),
    );
    Ok(())
}
