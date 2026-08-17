//! The two binding readers must not disagree. One names the accounts it wants,
//! the other takes the tenant; the review surface switched to the second one
//! purely for cost, so what pins that switch is that the answers match.
//!
//! INVARIANT: never `#[ignore]` these — the identity CI job runs `cargo test`
//! without `--include-ignored`, so an ignored case silently stops running.

use uuid::Uuid;

use crate::domain::login_bootstrap::LOGIN_BOOTSTRAP_REASON;
use crate::domain::seed::KnownBinding;

use super::resolution_repo::{current_bindings, current_bindings_in_tenant};
use super::test_fixture::{FIXTURE_REASON, fixture_or_skip};

type TestResult = anyhow::Result<()>;

#[tokio::test]
async fn naming_every_account_and_taking_the_tenant_answer_the_same() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let anna = f.person("anna@binding-reads.test").await?;
    let boris = f.person("boris@binding-reads.test").await?;
    let operator = f.person("operator@binding-reads.test").await?;
    // One of each shape the review surface distinguishes: automatic, rebound
    // later, operator-authored, minted at a sign-in. Each observation of a given
    // person gets its own age — the journal's uniqueness key ends in
    // `created_at`, so two of theirs at the same instant collide.
    let plain = f.bound_at("acct-plain", anna, FIXTURE_REASON, 95).await?;
    let rebound = f.bound_at("acct-rebound", anna, FIXTURE_REASON, 90).await?;
    f.bound_at("acct-rebound", boris, FIXTURE_REASON, 30)
        .await?;
    let decided = f
        .bound_by_operator_at("acct-decided", boris, operator, 60)
        .await?;
    let minted = f
        .bound_at("acct-minted", boris, LOGIN_BOOTSTRAP_REASON, 45)
        .await?;
    let asked = [plain, rebound, decided, minted];

    let by_name = current_bindings(&f.db, f.tenant, &asked).await?;
    let by_tenant = current_bindings_in_tenant(&f.db, f.tenant).await?;

    assert_eq!(by_name.len(), asked.len(), "every asked account was found");
    for account in &asked {
        assert_eq!(
            by_tenant.get(account),
            by_name.get(account),
            "the two readers disagree about {}",
            account.account_id
        );
    }
    Ok(())
}

#[tokio::test]
async fn the_tenant_read_carries_what_the_review_surface_reads() -> TestResult {
    // person, author and reason are what the queue classifies on: lose any of
    // them and a settled account reappears as undecided, or the other way round.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let person = f.person("holder@binding-reads.test").await?;
    let operator = f.person("operator@binding-reads.test").await?;
    let decided = f
        .bound_by_operator_at("acct-decided", person, operator, 60)
        .await?;
    let minted = f
        .bound_at("acct-minted", person, LOGIN_BOOTSTRAP_REASON, 45)
        .await?;

    let bindings = current_bindings_in_tenant(&f.db, f.tenant).await?;

    assert_eq!(
        bindings.get(&decided),
        Some(&KnownBinding {
            person_id: person,
            author_person_id: operator,
            provisioned_at_login: false,
        })
    );
    assert_eq!(
        bindings.get(&minted),
        Some(&KnownBinding {
            person_id: person,
            author_person_id: Uuid::nil(),
            provisioned_at_login: true,
        })
    );
    Ok(())
}

#[tokio::test]
async fn the_binding_in_force_is_the_latest_observed_not_the_last_written() -> TestResult {
    // A journal row may be backdated — the seed writes what a source reported,
    // not when it ran — so "the row with the highest id" is not the answer. A
    // reader that took it would hand an account back to a person it left.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let left = f.person("left@binding-reads.test").await?;
    let holds = f.person("holds@binding-reads.test").await?;
    let account = f
        .bound_at("acct-backdated", holds, FIXTURE_REASON, 30)
        .await?;
    f.bound_at("acct-backdated", left, FIXTURE_REASON, 3600)
        .await?;

    let by_tenant = current_bindings_in_tenant(&f.db, f.tenant).await?;
    let by_name = current_bindings(&f.db, f.tenant, std::slice::from_ref(&account)).await?;

    assert_eq!(
        by_tenant.get(&account).map(|b| b.person_id),
        Some(holds),
        "the newer observation holds the account"
    );
    assert_eq!(by_tenant.get(&account), by_name.get(&account));
    Ok(())
}

#[tokio::test]
async fn the_tenant_read_stops_at_the_tenant() -> TestResult {
    // The account key carries no tenant, so a neighbour's binding under the same
    // source and account id would silently answer for ours.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let neighbour = f.in_another_tenant();
    let ours = f.person("ours@binding-reads.test").await?;
    let theirs = neighbour.person("theirs@binding-reads.test").await?;
    let shared_id = format!("acct-shared-{}", Uuid::now_v7().simple());
    let account = f.bound_at(&shared_id, ours, FIXTURE_REASON, 60).await?;
    neighbour
        .bound_at(&shared_id, theirs, FIXTURE_REASON, 30)
        .await?;

    let bindings = current_bindings_in_tenant(&f.db, f.tenant).await?;

    assert_eq!(
        bindings.get(&account).map(|b| b.person_id),
        Some(ours),
        "the neighbour's newer binding must not answer for us"
    );
    Ok(())
}
