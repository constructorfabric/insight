//! The tenant's address → person map, against a live `MariaDB`.
//!
//! `latest_email_to_person` is the persons-seed's view of who already holds an
//! address, and the review surface reads it to tell a claimed address from a
//! free one. Both treat it as the tenant's whole current answer, so what has to
//! hold is the ranking, the tenant filter, and the normalization its callers
//! look keys up by.
//!
//! The seed's other reader, `known_account_bindings`, delegates to
//! `resolution_repo::current_bindings_in_tenant` — `binding_reads_live_tests`
//! pins that query, and a second copy of those cases would be a second answer to
//! maintain rather than a second thing proven.
//!
//! INVARIANT: never `#[ignore]` these — the identity CI job runs `cargo test`
//! without `--include-ignored`, so an ignored case silently stops running.
//!
//! Addresses carry the fixture's tenant: the fixture never cleans up and
//! `INTEGRATION_TESTS_MARIADB_URL` may name a shared `MariaDB`, so a fixed
//! address would make two concurrent runs race.

use super::seed_repo;
use super::test_fixture::fixture_or_skip;

type TestResult = anyhow::Result<()>;

#[tokio::test]
async fn an_address_is_owned_by_the_latest_person_to_state_it() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // A leaver's address handed to a joiner: two persons, one address, and only
    // the later observation may own it.
    let shared = format!("shared.{}@emailmap.test", f.tenant.simple());
    let leaver = f
        .person(&format!("leaver.{}@emailmap.test", f.tenant.simple()))
        .await?;
    let joiner = f
        .person(&format!("joiner.{}@emailmap.test", f.tenant.simple()))
        .await?;
    // INVARIANT: the row that must WIN is written FIRST. Written last, it would
    // also be the highest id, and the case could not tell the latest-observed
    // rule from "the row inserted last".
    f.observed_at(joiner, "email", &shared, 60).await?;
    f.observed_at(leaver, "email", &shared, 86_400).await?;

    let map = seed_repo::latest_email_to_person(&f.db, f.tenant).await?;

    assert_eq!(
        map.get(&shared),
        Some(&joiner),
        "the address must be owned by the person who stated it last"
    );
    Ok(())
}

#[tokio::test]
async fn an_address_is_keyed_in_lower_case() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let stated = format!("Ivan.Petrov.{}@EmailMap.test", f.tenant.simple());
    let person = f.person(&stated).await?;

    let map = seed_repo::latest_email_to_person(&f.db, f.tenant).await?;

    assert_eq!(
        map.get(&stated.to_lowercase()),
        Some(&person),
        "callers look the address up lowercased, so that is the key that must exist"
    );
    assert!(
        !map.contains_key(&stated),
        "keeping the case the source stated would give the same address two keys"
    );
    Ok(())
}

#[tokio::test]
async fn another_tenants_address_is_absent_from_this_tenants_map() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let other = f.in_another_tenant();
    let addr = format!("elsewhere.{}@emailmap.test", other.tenant.simple());
    let ours = format!("ours.{}@emailmap.test", f.tenant.simple());
    other.person(&addr).await?;
    let mine = f.person(&ours).await?;

    let map = seed_repo::latest_email_to_person(&f.db, f.tenant).await?;

    assert!(
        !map.contains_key(&addr),
        "the map stops at the tenant that asked for it"
    );
    assert_eq!(
        map.get(&ours),
        Some(&mine),
        "while this tenant's own address is in it"
    );
    Ok(())
}

#[tokio::test]
async fn an_address_the_source_left_blank_is_not_a_key() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let addr = format!("blank.{}@emailmap.test", f.tenant.simple());
    let person = f.person(&addr).await?;
    f.observed(person, "email", "").await?;

    let map = seed_repo::latest_email_to_person(&f.db, f.tenant).await?;

    assert!(
        !map.contains_key(""),
        "a blank address is an absent value, not an identity every blank row shares"
    );
    assert_eq!(
        map.get(&addr),
        Some(&person),
        "the blank row must not displace the address the person does hold"
    );
    Ok(())
}
