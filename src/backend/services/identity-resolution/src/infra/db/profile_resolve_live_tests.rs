//! What the profile lookups resolve to, against a live `MariaDB`.
//!
//! `POST /v1/profiles` dispatches on `value_type`, and the HTTP suite drives
//! only the `person_id` mode. The other two rank the journal in SQL and lean on
//! the `value_id` collation for case — neither can be established away from the
//! real schema, so these are the cases that pin them.
//!
//! INVARIANT: never `#[ignore]` these — the identity CI job runs `cargo test`
//! without `--include-ignored`, so an ignored case silently stops running.
//!
//! Addresses and account ids carry the fixture's tenant: the fixture never
//! cleans up and `INTEGRATION_TESTS_MARIADB_URL` may name a shared `MariaDB`, so
//! a fixed value would make two concurrent runs race.

use crate::domain::resolution::EXCLUDED_PERSON;

use super::persons_repo;
use super::test_fixture::{SOURCE_TYPE, fixture_or_skip};

type TestResult = anyhow::Result<()>;

/// A second source stating values under the same instance — for the cases where
/// WHICH source said it is the point.
const CHAT: &str = "zulip-proxy";

#[tokio::test]
async fn an_address_resolves_in_whatever_case_it_is_asked_for() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let addr = format!("holder.{}@profiles.test", f.tenant.simple());
    let person = f.person(&addr).await?;

    // A caller hands over whatever its directory holds, and the endpoint folds
    // nothing itself: the trim is Rust's, the case is the column's collation.
    for asked in [addr.clone(), addr.to_uppercase(), format!("  {addr}  ")] {
        assert_eq!(
            persons_repo::resolve_person_ids_by_email(&f.db, f.tenant, &asked).await?,
            vec![person],
            "{asked:?} must reach the same person",
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_address_nobody_stated_resolves_nobody() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // The tenant holds an address, so an empty answer here can only come from
    // the predicate declining the one it was asked for.
    let stated = format!("stated.{}@profiles.test", f.tenant.simple());
    let absent = format!("absent.{}@profiles.test", f.tenant.simple());
    let person = f.person(&stated).await?;

    assert_eq!(
        persons_repo::resolve_person_ids_by_email(&f.db, f.tenant, &stated).await?,
        vec![person],
    );
    assert!(
        persons_repo::resolve_person_ids_by_email(&f.db, f.tenant, &absent)
            .await?
            .is_empty(),
        "an address the journal never carried must resolve nobody"
    );
    Ok(())
}

#[tokio::test]
async fn another_tenants_address_resolves_nobody_here() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let other = f.in_another_tenant();
    let addr = format!("elsewhere.{}@profiles.test", other.tenant.simple());
    let theirs = other.person(&addr).await?;
    let ours = f
        .person(&format!("ours.{}@profiles.test", f.tenant.simple()))
        .await?;

    assert!(
        persons_repo::resolve_person_ids_by_email(&f.db, f.tenant, &addr)
            .await?
            .is_empty(),
        "a person another tenant states must not resolve here"
    );
    assert_eq!(
        persons_repo::resolve_person_ids_by_email(&other.db, other.tenant, &addr).await?,
        vec![theirs],
        "and they must still resolve where they were stated"
    );
    assert_eq!(
        persons_repo::resolve_person_ids_by_email(
            &f.db,
            f.tenant,
            &format!("ours.{}@profiles.test", f.tenant.simple())
        )
        .await?,
        vec![ours],
    );
    Ok(())
}

#[tokio::test]
async fn a_replaced_address_no_longer_resolves_its_owner() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // INVARIANT: the row that must WIN is written FIRST. Written last, it would
    // also be the highest id, and the case could not tell the latest-observed
    // rule from "the row inserted last".
    let former = format!("former.{}@profiles.test", f.tenant.simple());
    let current = format!("current.{}@profiles.test", f.tenant.simple());
    let person = f.person(&current).await?;
    f.observed_at(person, "email", &former, 86_400).await?;

    assert!(
        persons_repo::resolve_person_ids_by_email(&f.db, f.tenant, &former)
            .await?
            .is_empty(),
        "the superseded address must resolve nobody"
    );
    assert_eq!(
        persons_repo::resolve_person_ids_by_email(&f.db, f.tenant, &current).await?,
        vec![person],
        "the address in force must still resolve its owner"
    );
    Ok(())
}

#[tokio::test]
async fn two_persons_stating_one_address_are_both_returned() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let shared = format!("shared.{}@profiles.test", f.tenant.simple());
    let first = f.person(&shared).await?;
    let second = f.person(&shared).await?;

    let mut got = persons_repo::resolve_person_ids_by_email(&f.db, f.tenant, &shared).await?;
    got.sort();
    let mut want = vec![first, second];
    want.sort();

    assert_eq!(
        got, want,
        "an address two persons state is a conflict to report, not one to silently pick from"
    );
    Ok(())
}

#[tokio::test]
async fn the_excluded_sentinel_never_resolves_as_a_person() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let addr = format!("excluded.{}@profiles.test", f.tenant.simple());
    let real = format!("real.{}@profiles.test", f.tenant.simple());
    f.person_as(EXCLUDED_PERSON, &addr).await?;
    let person = f.person(&real).await?;

    assert!(
        persons_repo::resolve_person_ids_by_email(&f.db, f.tenant, &addr)
            .await?
            .is_empty(),
        "the exclusion sentinel is not a person any lookup may answer with"
    );
    assert_eq!(
        persons_repo::resolve_person_ids_by_email(&f.db, f.tenant, &real).await?,
        vec![person],
        "while a person stated the same way still resolves"
    );
    Ok(())
}

#[tokio::test]
async fn a_source_native_id_resolves_the_person_holding_it() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let account = format!("acct-{}", f.tenant.simple());
    let person = f
        .person(&format!("byid.{}@profiles.test", f.tenant.simple()))
        .await?;
    f.observed(person, "id", &account).await?;

    assert_eq!(
        persons_repo::resolve_person_ids_by_source_id(
            &f.db,
            f.tenant,
            SOURCE_TYPE,
            f.source_id,
            &account
        )
        .await?,
        vec![person],
    );
    Ok(())
}

#[tokio::test]
async fn an_id_only_another_source_stated_resolves_nobody_under_this_one() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // One account id, stated under one source type only. Both lookups name the
    // same instance, so the source TYPE is all that tells them apart.
    let account = format!("acct-chat-{}", f.tenant.simple());
    let person = f
        .person(&format!("chatter.{}@profiles.test", f.tenant.simple()))
        .await?;
    f.observed_from(CHAT, person, "id", &account).await?;

    assert!(
        persons_repo::resolve_person_ids_by_source_id(
            &f.db,
            f.tenant,
            SOURCE_TYPE,
            f.source_id,
            &account
        )
        .await?
        .is_empty(),
        "an id is answered only for the source that stated it"
    );
    assert_eq!(
        persons_repo::resolve_person_ids_by_source_id(&f.db, f.tenant, CHAT, f.source_id, &account)
            .await?,
        vec![person],
        "the source that stated it must still resolve it"
    );
    Ok(())
}

#[tokio::test]
async fn an_id_another_instance_of_the_same_source_stated_resolves_its_own_person() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // Two installs of one system under one tenant: vendor account ids collide
    // across hosts, so the id alone cannot say which person is meant.
    let other = f.in_another_source_instance();
    let account = format!("acct-shared-{}", f.tenant.simple());
    let mine = f
        .person(&format!("mine.{}@profiles.test", f.tenant.simple()))
        .await?;
    let theirs = other
        .person(&format!("theirs.{}@profiles.test", f.tenant.simple()))
        .await?;
    f.observed(mine, "id", &account).await?;
    other.observed(theirs, "id", &account).await?;

    assert_eq!(
        persons_repo::resolve_person_ids_by_source_id(
            &f.db,
            f.tenant,
            SOURCE_TYPE,
            f.source_id,
            &account
        )
        .await?,
        vec![mine],
        "each instance must answer with the person IT stated the id for"
    );
    assert_eq!(
        persons_repo::resolve_person_ids_by_source_id(
            &f.db,
            f.tenant,
            SOURCE_TYPE,
            other.source_id,
            &account
        )
        .await?,
        vec![theirs],
    );
    Ok(())
}

#[tokio::test]
async fn another_tenants_account_id_resolves_nobody_here() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // The same source INSTANCE serving two tenants — what `in_another_tenant`
    // models by carrying the source id over. Only the tenant tells the two
    // accounts apart. This is the `value_type='id'` arm of POST /v1/profiles;
    // the login bootstrap resolves through `resolve_person_id_by_source_any_tenant`,
    // which is deliberately tenant-agnostic and documents why.
    let other = f.in_another_tenant();
    let account = format!("acct-tenant-{}", f.tenant.simple());
    let theirs = other
        .person(&format!("theirs.{}@profiles.test", other.tenant.simple()))
        .await?;
    other.observed(theirs, "id", &account).await?;

    assert!(
        persons_repo::resolve_person_ids_by_source_id(
            &f.db,
            f.tenant,
            SOURCE_TYPE,
            f.source_id,
            &account
        )
        .await?
        .is_empty(),
        "an account another tenant holds must resolve nobody here"
    );
    assert_eq!(
        persons_repo::resolve_person_ids_by_source_id(
            &other.db,
            other.tenant,
            SOURCE_TYPE,
            other.source_id,
            &account
        )
        .await?,
        vec![theirs],
        "and it must still resolve for the tenant that holds it"
    );
    Ok(())
}

#[tokio::test]
async fn a_superseded_id_no_longer_resolves_its_owner() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let former = format!("acct-former-{}", f.tenant.simple());
    let current = format!("acct-current-{}", f.tenant.simple());
    let person = f
        .person(&format!("moved.{}@profiles.test", f.tenant.simple()))
        .await?;
    // INVARIANT: the row that must WIN is written FIRST — see the address case.
    f.observed(person, "id", &current).await?;
    f.observed_at(person, "id", &former, 86_400).await?;

    assert!(
        persons_repo::resolve_person_ids_by_source_id(
            &f.db,
            f.tenant,
            SOURCE_TYPE,
            f.source_id,
            &former
        )
        .await?
        .is_empty(),
        "the id the account no longer carries must resolve nobody"
    );
    assert_eq!(
        persons_repo::resolve_person_ids_by_source_id(
            &f.db,
            f.tenant,
            SOURCE_TYPE,
            f.source_id,
            &current
        )
        .await?,
        vec![person],
    );
    Ok(())
}
