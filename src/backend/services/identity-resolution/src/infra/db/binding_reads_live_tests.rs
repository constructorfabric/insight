use uuid::Uuid;

use crate::domain::login_bootstrap::LOGIN_BOOTSTRAP_REASON;
use crate::domain::provenance::Provenance;
use crate::domain::seed::KnownBinding;

use super::resolution_repo::{Ceiling, LOOKUP_CHUNK, current_bindings, current_bindings_in_tenant};
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
    let unasked = f.bound_at("acct-unasked", anna, FIXTURE_REASON, 20).await?;
    let asked = [plain, rebound, decided, minted];

    let by_name = current_bindings(&f.db, f.tenant, &asked).await?;
    let by_tenant = current_bindings_in_tenant(&f.db, f.tenant, Ceiling::Unbounded)
        .await?
        .by_account;

    assert_eq!(by_name.len(), asked.len(), "every asked account was found");
    for account in &asked {
        assert_eq!(
            by_tenant.get(account),
            by_name.get(account),
            "the two readers disagree about {}",
            account.account_id
        );
    }
    assert!(
        by_tenant.contains_key(&unasked) && !by_name.contains_key(&unasked),
        "the tenant read is the wider one — otherwise the two agreed vacuously"
    );
    Ok(())
}

#[tokio::test]
async fn asking_about_more_accounts_than_one_statement_carries_answers_for_all_of_them()
-> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let holder = f.person("holder@binding-reads.test").await?;
    let mut asked = Vec::new();
    for i in 0..(LOOKUP_CHUNK + 5) {
        asked.push(
            f.bound_at(
                &format!("acct-batch-{i:04}"),
                holder,
                FIXTURE_REASON,
                u32::try_from(i).unwrap_or(u32::MAX) + 1,
            )
            .await?,
        );
    }

    let bindings = current_bindings(&f.db, f.tenant, &asked).await?;

    assert_eq!(
        bindings.len(),
        asked.len(),
        "a batch boundary dropped accounts from the answer"
    );
    Ok(())
}

#[tokio::test]
async fn a_read_that_ends_exactly_on_the_ceiling_is_not_truncated() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let holder = f.person("holder@binding-reads.test").await?;
    for i in 0..3u32 {
        f.bound_at(&format!("acct-ceil-{i}"), holder, FIXTURE_REASON, i + 1)
            .await?;
    }

    for (ceiling, expected) in [(2, true), (3, false), (4, false)] {
        let read = current_bindings_in_tenant(&f.db, f.tenant, Ceiling::Bounded(ceiling)).await?;

        assert_eq!(
            read.truncated, expected,
            "three bindings read under a ceiling of {ceiling}"
        );
        assert_eq!(
            read.by_account.len(),
            usize::try_from(ceiling.min(3)).unwrap_or(3),
            "a ceiling of {ceiling} served the wrong number of rows"
        );
    }
    Ok(())
}

#[tokio::test]
async fn the_tenant_read_carries_what_the_review_surface_reads() -> TestResult {
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

    let bindings = current_bindings_in_tenant(&f.db, f.tenant, Ceiling::Unbounded)
        .await?
        .by_account;

    assert_eq!(
        bindings.get(&decided),
        Some(&KnownBinding {
            person_id: person,
            author_person_id: operator,
            provenance: Provenance::Resolved,
        })
    );
    assert_eq!(
        bindings.get(&minted),
        Some(&KnownBinding {
            person_id: person,
            author_person_id: Uuid::nil(),
            provenance: Provenance::LoginBootstrap,
        })
    );
    Ok(())
}

#[tokio::test]
async fn the_binding_in_force_is_the_latest_observed_not_the_last_written() -> TestResult {
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

    let by_tenant = current_bindings_in_tenant(&f.db, f.tenant, Ceiling::Unbounded)
        .await?
        .by_account;
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
async fn both_readers_stop_at_the_tenant() -> TestResult {
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

    let by_tenant = current_bindings_in_tenant(&f.db, f.tenant, Ceiling::Unbounded)
        .await?
        .by_account;
    let by_name = current_bindings(&f.db, f.tenant, std::slice::from_ref(&account)).await?;

    for (reader, bindings) in [("tenant-wide", &by_tenant), ("by-name", &by_name)] {
        assert_eq!(
            bindings.get(&account).map(|b| b.person_id),
            Some(ours),
            "the {reader} reader let a neighbour's newer binding answer for us"
        );
    }
    Ok(())
}
