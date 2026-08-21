use uuid::Uuid;

use super::person_listing::{self, After, Listed, PersonListRow, Restrict, VisibleTo};
use super::test_fixture::{SOURCE_TYPE, fixture_or_skip};
use crate::config::VisibilityPolicy;

type TestResult = anyhow::Result<()>;

const PAGE: u64 = 50;

/// The roster as these cases read it: every identity, so a visibility case is
/// about visibility alone. The endpoint asks for account holders — the fixture's
/// `person()` carries no canonical binding, so requiring one here would empty
/// every case below and prove nothing about the visible set.
/// [`only_account_holders`] covers the requirement itself.
async fn page(
    f: &super::test_fixture::Fixture,
    viewer: Uuid,
    policy: VisibilityPolicy,
    terms: &[String],
    after: Option<After<'_>>,
    limit: u64,
) -> anyhow::Result<Vec<PersonListRow>> {
    listed_page(
        f,
        viewer,
        policy,
        terms,
        after,
        limit,
        Listed::EveryIdentity,
    )
    .await
}

/// The roster exactly as the endpoint asks for it.
async fn only_account_holders(
    f: &super::test_fixture::Fixture,
    viewer: Uuid,
    limit: u64,
) -> anyhow::Result<Vec<PersonListRow>> {
    listed_page(
        f,
        viewer,
        VisibilityPolicy::Flat,
        &[],
        None,
        limit,
        Listed::AccountHolders,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn listed_page(
    f: &super::test_fixture::Fixture,
    viewer: Uuid,
    policy: VisibilityPolicy,
    terms: &[String],
    after: Option<After<'_>>,
    limit: u64,
    listed: Listed,
) -> anyhow::Result<Vec<PersonListRow>> {
    person_listing::list_persons(
        &f.db,
        f.tenant,
        terms,
        &[],
        Restrict {
            listed,
            visible_to: Some(VisibleTo {
                viewer_person_id: viewer,
                org_source_type: SOURCE_TYPE,
                policy,
            }),
        },
        after,
        limit,
    )
    .await
}

fn ids(rows: &[PersonListRow]) -> Vec<Uuid> {
    rows.iter().map(|row| row.person_id).collect()
}

#[tokio::test]
async fn a_flat_roster_lists_every_person_of_the_tenant() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let viewer = f.person("aaa@roster.test").await?;
    let second = f.person("bbb@roster.test").await?;
    let third = f.person("ccc@roster.test").await?;

    let rows = page(&f, viewer, VisibilityPolicy::Flat, &[], None, PAGE).await?;

    assert_eq!(ids(&rows), vec![viewer, second, third]);
    Ok(())
}

#[tokio::test]
async fn an_org_chart_roster_stops_at_the_reporting_line() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let lead = f.person("lead@roster.test").await?;
    let report = f.person("report@roster.test").await?;
    let stranger = f.person("stranger@roster.test").await?;
    f.reports_to(report, lead).await?;

    let rows = page(&f, lead, VisibilityPolicy::OrgChart, &[], None, PAGE).await?;

    assert_eq!(ids(&rows), vec![lead, report], "never {stranger}");
    Ok(())
}

#[tokio::test]
async fn a_roster_lists_another_tenants_person_under_no_policy() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let viewer = f.person("boundary@roster.test").await?;
    let elsewhere = f.in_another_tenant();
    let foreign = elsewhere.person("foreign@roster.test").await?;

    for policy in [VisibilityPolicy::OrgChart, VisibilityPolicy::Flat] {
        let rows = page(&f, viewer, policy, &[], None, PAGE).await?;

        assert!(
            !ids(&rows).contains(&foreign),
            "{policy:?} crossed the tenant boundary"
        );
    }
    Ok(())
}

#[tokio::test]
async fn the_roster_is_ordered_by_label_and_a_cursor_resumes_after_it() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // The label prefers a display name, so the order must ignore the address.
    let carol = f.person("zzz-1@roster.test").await?;
    f.observed(carol, "display_name", "Carol").await?;
    let alice = f.person("zzz-2@roster.test").await?;
    f.observed(alice, "display_name", "Alice").await?;
    let bob = f.person("zzz-3@roster.test").await?;
    f.observed(bob, "display_name", "Bob").await?;

    let first = page(&f, alice, VisibilityPolicy::Flat, &[], None, 2).await?;
    assert_eq!(ids(&first), vec![alice, bob], "alphabetical by label");

    let last = first.last().ok_or_else(|| anyhow::anyhow!("empty page"))?;
    let next = page(
        &f,
        alice,
        VisibilityPolicy::Flat,
        &[],
        Some(After {
            order_key: &last.order_key,
            person_id: last.person_id,
        }),
        2,
    )
    .await?;

    assert_eq!(ids(&next), vec![carol], "the page after it, no overlap");
    Ok(())
}

#[tokio::test]
async fn a_person_the_log_knows_only_by_an_account_id_is_still_listed() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let named = f.person("named@roster.test").await?;
    let account_only = f.emailless_person().await?;

    let rows = page(&f, named, VisibilityPolicy::Flat, &[], None, PAGE).await?;

    let listed = ids(&rows);
    assert!(listed.contains(&account_only), "missing {account_only}");
    assert_eq!(
        listed.last(),
        Some(&account_only),
        "a person with no label sorts after every named one"
    );
    Ok(())
}

#[tokio::test]
async fn a_search_term_narrows_the_roster_within_the_visible_set() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let viewer = f.person("searchable@roster.test").await?;
    let other = f.person("elsewhere@roster.test").await?;
    f.observed(viewer, "display_name", "Findable Person")
        .await?;
    f.observed(other, "display_name", "Someone Else").await?;

    let rows = page(
        &f,
        viewer,
        VisibilityPolicy::Flat,
        &["findable".to_owned()],
        None,
        PAGE,
    )
    .await?;

    assert_eq!(ids(&rows), vec![viewer]);
    Ok(())
}

#[tokio::test]
async fn a_roster_of_account_holders_leaves_out_an_identity_nobody_claims() -> TestResult {
    // The shape a git-only organisation is full of: an address a commit carried,
    // which the journal turned into a person because that is what the journal
    // does. Nobody here holds it — no connector ever claimed it as an account —
    // so it is an observation, and a roster that lists it is a directory of
    // strangers. The account holder beside it is what the roster IS.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let observed_only = f.person("commit-author@roster.test").await?;
    let account_holder = f.emailless_person().await?;

    let listed = ids(&only_account_holders(&f, account_holder, PAGE).await?);

    assert!(
        listed.contains(&account_holder),
        "the account holder must be listed: {listed:?}"
    );
    assert!(
        !listed.contains(&observed_only),
        "an unclaimed address must not reach the roster: {listed:?}"
    );
    Ok(())
}
