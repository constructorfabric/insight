//! INVARIANT: never `#[ignore]` these — the identity CI job runs `cargo test`
//! without `--include-ignored`, so an ignored case silently stops running.

use uuid::Uuid;

use super::test_fixture::fixture_or_skip;
use super::{persons_repo, roles_repo, subchart_repo};

type TestResult = anyhow::Result<()>;

#[tokio::test]
async fn caller_without_reports_still_sees_themselves() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let leaf = f.person("leaf@visible-set.test").await?;
    let stranger = f.person("stranger@visible-set.test").await?;

    let visible = f.visible(leaf, &[leaf, stranger]).await?;

    assert_eq!(visible, vec![leaf], "self is visible, the stranger is not");
    Ok(())
}

#[tokio::test]
async fn manager_sees_a_transitive_descendant_but_not_an_unrelated_person() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let top = f.person("top@visible-set.test").await?;
    let mid = f.person("mid@visible-set.test").await?;
    let deep = f.person("deep@visible-set.test").await?;
    let stranger = f.person("stranger@visible-set.test").await?;
    f.reports_to(mid, top).await?;
    f.reports_to(deep, mid).await?;

    let visible = f.visible(top, &[deep, stranger]).await?;

    assert_eq!(
        visible,
        vec![deep],
        "descent is transitive, and stops there"
    );
    Ok(())
}

#[tokio::test]
async fn an_explicit_grant_reaches_outside_the_reporting_line() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let viewer = f.person("viewer@visible-set.test").await?;
    let granted = f.person("granted@visible-set.test").await?;
    let stranger = f.person("stranger@visible-set.test").await?;
    f.grant(viewer, Some(granted)).await?;

    let visible = f.visible(viewer, &[granted, stranger]).await?;

    assert_eq!(visible, vec![granted]);
    Ok(())
}

#[tokio::test]
async fn the_wildcard_echo_is_bounded_by_the_tenant() -> TestResult {
    // A wildcard grant covers everyone IN THE TENANT: an id from another
    // tenant or from nowhere must not come back from the batch existence
    // filter, or the visible-persons wildcard branch would confirm foreign
    // UUIDs as visible — and analytics reads that answer as authorization.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let ours = f.person("in-tenant@example.com").await?;

    let other = f.in_another_tenant();
    let foreign = other.person("other-tenant@example.com").await?;

    let got =
        persons_repo::persons_in_tenant(&f.db, f.tenant, &[ours, foreign, Uuid::now_v7()]).await?;

    assert_eq!(got, vec![ours], "only the caller-tenant person survives");
    Ok(())
}

#[tokio::test]
async fn a_person_exists_only_inside_their_own_tenant() -> TestResult {
    // The existence probe behind `value_type='person_id'`: a person another
    // tenant observed must read as unknown here, or the profile lookup would
    // answer 404-vs-200 differently depending on data the caller cannot see.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let ours = f.person("exists@visible-set.test").await?;
    let foreign = f
        .in_another_tenant()
        .person("foreign@visible-set.test")
        .await?;

    assert!(
        persons_repo::person_exists(&f.db, f.tenant, ours).await?,
        "a person the tenant observed exists"
    );
    assert!(
        !persons_repo::person_exists(&f.db, f.tenant, foreign).await?,
        "another tenant's person does not"
    );
    assert!(
        !persons_repo::person_exists(&f.db, f.tenant, Uuid::now_v7()).await?,
        "an id nobody observed does not"
    );
    Ok(())
}

#[tokio::test]
async fn a_person_with_no_email_still_exists() -> TestResult {
    // The person_id key exists to reach exactly this person; an existence probe
    // that keyed on the email observation would report them missing.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let person = f.emailless_person().await?;

    assert!(persons_repo::person_exists(&f.db, f.tenant, person).await?);
    Ok(())
}

#[tokio::test]
async fn a_wildcard_grant_covers_the_whole_tenant() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let viewer = f.person("viewer@visible-set.test").await?;
    let unrelated = f.person("unrelated@visible-set.test").await?;
    f.grant(viewer, None).await?;

    assert!(
        subchart_repo::has_wildcard_grant(&f.db, f.tenant, viewer).await?,
        "the probe must see the wildcard grant"
    );
    let mut visible = f.visible(viewer, &[unrelated]).await?;
    visible.sort();
    assert_eq!(
        visible,
        vec![unrelated],
        "the CTE's wildcard arm agrees with the probe"
    );
    Ok(())
}

#[tokio::test]
async fn the_admin_role_confers_no_visibility() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let admin = f.person("admin@visible-set.test").await?;
    let stranger = f.person("stranger@visible-set.test").await?;
    f.make_admin(admin).await?;

    assert!(
        roles_repo::has_active_role(&f.db, f.tenant, admin, roles_repo::ADMIN_ROLE_ID).await?,
        "the fixture really did grant the admin role"
    );
    assert!(
        !subchart_repo::has_wildcard_grant(&f.db, f.tenant, admin).await?,
        "the role is not a grant"
    );
    assert_eq!(
        f.visible(admin, &[stranger]).await?,
        Vec::<Uuid>::new(),
        "administering identity must not widen who you can see"
    );
    Ok(())
}

#[tokio::test]
async fn a_flat_policy_shows_a_caller_with_no_reports_the_whole_tenant() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let leaf = f.person("leaf@flat-policy.test").await?;
    let stranger = f.person("stranger@flat-policy.test").await?;

    let mut visible = f.visible_flat(leaf, &[leaf, stranger]).await?;
    visible.sort();
    let mut expected = vec![leaf, stranger];
    expected.sort();

    assert_eq!(visible, expected);
    Ok(())
}

#[tokio::test]
async fn a_flat_policy_covers_the_tenants_persons_not_every_id_asked_about() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let viewer = f.person("viewer@flat-policy.test").await?;
    let elsewhere = f.in_another_tenant();
    let foreign = elsewhere.person("foreign@flat-policy.test").await?;
    let never_a_person = Uuid::now_v7();

    let visible = f
        .visible_flat(viewer, &[viewer, foreign, never_a_person])
        .await?;

    assert_eq!(visible, vec![viewer]);
    Ok(())
}

#[tokio::test]
async fn a_flat_policy_answers_the_single_target_probe_within_the_tenant() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let viewer = f.person("probe-viewer@flat-policy.test").await?;
    let stranger = f.person("probe-stranger@flat-policy.test").await?;
    let elsewhere = f.in_another_tenant();
    let foreign = elsewhere.person("probe-foreign@flat-policy.test").await?;

    assert!(f.can_see_flat(viewer, stranger).await?);
    assert!(!f.can_see_flat(viewer, foreign).await?);
    Ok(())
}

#[tokio::test]
async fn a_wildcard_grant_confirms_only_persons_of_the_tenant() -> TestResult {
    // A wildcard grant covers everyone IN THE TENANT. Confirming an id from
    // another tenant — or one that names nobody — would answer about a person
    // the grant never reached, and `GET /v1/subchart/{id}` reads this predicate
    // to decide between a subtree and a 404.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let viewer = f.person("wildcard-viewer@visible-set.test").await?;
    let ours = f.person("wildcard-ours@visible-set.test").await?;
    let elsewhere = f.in_another_tenant();
    let foreign = elsewhere
        .person("wildcard-foreign@visible-set.test")
        .await?;
    f.grant(viewer, None).await?;

    assert!(
        f.can_see(viewer, ours).await?,
        "the grant covers the tenant"
    );
    assert!(
        !f.can_see(viewer, foreign).await?,
        "{foreign} belongs to another tenant"
    );
    assert!(
        !f.can_see(viewer, Uuid::now_v7()).await?,
        "an id that names nobody is not a person the grant reaches"
    );
    Ok(())
}
