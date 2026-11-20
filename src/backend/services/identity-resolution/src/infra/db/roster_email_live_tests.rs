use super::persons_repo;
use super::test_fixture::{SOURCE_TYPE, fixture_or_skip};
use crate::domain::resolution::EXCLUDED_PERSON;

type TestResult = anyhow::Result<()>;

const CHAT: &str = "zulip-proxy";

#[tokio::test]
async fn an_address_resolves_whatever_case_the_token_carries_it_in() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let addr = format!("ivan.petrov.{}@vz.com", f.tenant.simple());
    let person = f.account_holder(&addr).await?;

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
    let alias = format!("sales.{}@vz.com", f.tenant.simple());
    let leaver = f
        .account_holder(&format!("leaver.{}@vz.com", f.tenant.simple()))
        .await?;
    f.observed_at(leaver, "email", &alias, 172_800).await?;
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
