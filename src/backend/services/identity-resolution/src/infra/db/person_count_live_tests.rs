//! The person total the console prints and the listing it sits above must
//! describe the same set — one is a `COUNT`, the other a keyset walk, and only a
//! live case can catch them drifting apart.
//!
//! INVARIANT: never `#[ignore]` these — the identity CI job runs `cargo test`
//! without `--include-ignored`, so an ignored case silently stops running.

use uuid::Uuid;

use crate::domain::resolution::EXCLUDED_PERSON;

use super::person_listing::{count_persons, list_persons};
use super::test_fixture::fixture_or_skip;

type TestResult = anyhow::Result<()>;

async fn browsed(f: &super::test_fixture::Fixture) -> anyhow::Result<usize> {
    Ok(list_persons(&f.db, f.tenant, &[], &[], None, None, 1_000)
        .await?
        .len())
}

#[tokio::test]
async fn the_total_counts_exactly_the_persons_the_listing_browses() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    f.person("first@person-count.test").await?;
    f.person("second@person-count.test").await?;
    // A person the journal knows by an id alone still browses and still counts.
    f.emailless_person().await?;

    let total = count_persons(&f.db, f.tenant).await?;

    assert_eq!(total, 3);
    assert_eq!(total, browsed(&f).await?, "the total is the list's length");
    Ok(())
}

#[tokio::test]
async fn observing_the_same_person_twice_still_counts_one() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let person = f.person("renamed@person-count.test").await?;
    f.observed(person, "email", "renamed-again@person-count.test")
        .await?;
    f.observed(person, "display_name", "Renamed Person").await?;

    let total = count_persons(&f.db, f.tenant).await?;

    assert_eq!(total, 1, "a person is one row's subject, not one row");
    assert_eq!(total, browsed(&f).await?);
    Ok(())
}

#[tokio::test]
async fn another_tenants_persons_are_not_in_the_total() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    f.person("ours@person-count.test").await?;
    let elsewhere = f.in_another_tenant();
    elsewhere.person("theirs@person-count.test").await?;

    assert_eq!(count_persons(&f.db, f.tenant).await?, 1);
    assert_eq!(count_persons(&f.db, elsewhere.tenant).await?, 1);
    Ok(())
}

#[tokio::test]
async fn the_excluded_sentinel_is_not_counted_as_a_person() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    f.person("real@person-count.test").await?;
    f.person_as(EXCLUDED_PERSON, "excluded@person-count.test")
        .await?;

    let total = count_persons(&f.db, f.tenant).await?;

    assert_eq!(total, 1, "the sentinel is a bucket, not a person");
    assert_eq!(total, browsed(&f).await?);
    Ok(())
}

#[tokio::test]
async fn an_empty_tenant_counts_none() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };

    assert_eq!(count_persons(&f.db, Uuid::now_v7()).await?, 0);
    Ok(())
}
