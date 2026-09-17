use chrono::TimeZone as _;
use serde_json::json;

use super::*;
use crate::domain::datasets::LEASE_SECS;

type R = Result<(), Box<dyn std::error::Error>>;

fn at(second: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 16, 12, 0, 0)
        .single()
        .unwrap_or_else(|| panic!("the fixture time exists"))
        + chrono::TimeDelta::seconds(second)
}

fn name(value: &str) -> DefinitionName {
    DefinitionName::parse(value).unwrap_or_else(|error| panic!("the name parses: {error}"))
}

fn declaration() -> Value {
    json!({ "fields": [{ "name": "day", "path": "day", "type": "string" }] })
}

#[tokio::test]
async fn a_create_claims_a_name_nobody_holds_and_keeps_what_it_was_given() -> R {
    let store = MemoryDatasets::at(at(0));

    let attempt = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();

    let Some(held) = store.get(&name("commits")).await? else {
        panic!("the claim left a row");
    };
    assert_eq!(held.state, DatasetState::Claimed);
    assert_eq!(held.name.as_str(), "commits");
    assert_eq!(held.declaration, declaration());
    assert_eq!(held.physical_table, None, "no table is provisioned yet");
    assert_eq!(
        held.held.map(|held| held.token),
        Some(attempt.token),
        "the row records the attempt that holds it"
    );
    assert_eq!(store.list().await?, vec!["commits".to_owned()]);

    Ok(())
}

#[tokio::test]
async fn a_second_create_is_refused_while_the_first_still_holds_the_lease() -> R {
    let store = MemoryDatasets::at(at(0));
    store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();

    let refused = store.take_create(&name("commits"), &declaration()).await;

    assert!(
        matches!(
            refused,
            Err(DatasetStoreError::Refused(Refused::Busy(Operation::Create)))
        ),
        "{refused:?}"
    );

    Ok(())
}

#[tokio::test]
async fn once_the_lease_lapses_another_attempt_may_take_the_dataset_over() -> R {
    let store = MemoryDatasets::at(at(0));
    let first = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();

    store.set_now(at(LEASE_SECS + 1));
    let second = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();

    assert_ne!(
        second.token, first.token,
        "the second attempt owns the dataset under its own token"
    );
    assert_ne!(
        second.token.table(&name("commits")),
        first.token.table(&name("commits")),
        "and provisions a table the first cannot drop"
    );

    Ok(())
}

#[tokio::test]
async fn a_dataset_that_was_never_there_has_nothing_to_remove() -> R {
    let store = MemoryDatasets::at(at(0));

    let refused = store.take_remove(&name("commits")).await;

    assert!(
        matches!(refused, Err(DatasetStoreError::Refused(Refused::Gone))),
        "{refused:?}"
    );

    Ok(())
}

#[tokio::test]
async fn a_name_under_removal_is_not_free_to_create_again() -> R {
    let store = MemoryDatasets::at(at(0));
    store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();
    store.set_now(at(LEASE_SECS + 1));
    store.take_remove(&name("commits")).await?;

    store.set_now(at(2 * LEASE_SECS + 2));
    let refused = store.take_create(&name("commits"), &declaration()).await;

    assert!(
        matches!(refused, Err(DatasetStoreError::Refused(Refused::Removing))),
        "{refused:?}"
    );

    Ok(())
}

#[tokio::test]
async fn an_attempt_that_kept_the_dataset_publishes_what_it_made() -> R {
    let store = MemoryDatasets::at(at(0));
    let attempt = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();
    let table = attempt.token.table(&name("commits"));

    let recorded = store
        .finish(
            &name("commits"),
            &attempt.token,
            Finish::Provisioned(table.clone()),
        )
        .await?;
    let published = store
        .finish(&name("commits"), &attempt.token, Finish::Ready)
        .await?;

    assert_eq!(recorded, Owning::Held);
    assert_eq!(published, Owning::Held);
    let Some(ready) = store.get(&name("commits")).await? else {
        panic!("the dataset stands");
    };
    assert_eq!(ready.state, DatasetState::Ready);
    assert_eq!(ready.physical_table, Some(table));
    assert!(ready.held.is_none(), "the operation is released");

    Ok(())
}

#[tokio::test]
async fn an_attempt_that_lost_the_dataset_publishes_nothing() -> R {
    let store = MemoryDatasets::at(at(0));
    let abandoned = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();

    store.set_now(at(LEASE_SECS + 1));
    let took_over = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();
    let stale = store
        .finish(&name("commits"), &abandoned.token, Finish::Ready)
        .await?;

    assert_eq!(stale, Owning::Lost);
    let Some(held) = store.get(&name("commits")).await? else {
        panic!("the dataset is still claimed");
    };
    assert_eq!(
        held.state,
        DatasetState::Claimed,
        "a stale attempt must not publish a dataset the live one is still making"
    );
    assert_eq!(held.held.map(|held| held.token), Some(took_over.token));

    Ok(())
}

#[tokio::test]
async fn taking_over_an_abandoned_create_publishes_its_own_declaration() -> R {
    let store = MemoryDatasets::at(at(0));
    store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();

    store.set_now(at(LEASE_SECS + 1));
    let second = json!({ "fields": [{ "name": "hour", "path": "hour", "type": "string" }] });
    let attempt = store
        .take_create(&name("commits"), &second)
        .await?
        .attempt();
    store
        .finish(&name("commits"), &attempt.token, Finish::Ready)
        .await?;

    let Some(ready) = store.get(&name("commits")).await? else {
        panic!("the dataset stands");
    };
    assert_eq!(ready.declaration, second);

    Ok(())
}

#[tokio::test]
async fn a_removal_that_kept_the_dataset_takes_the_row_with_it() -> R {
    let store = MemoryDatasets::at(at(0));
    let created = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();
    store
        .finish(&name("commits"), &created.token, Finish::Ready)
        .await?;

    let removal = store.take_remove(&name("commits")).await?;
    let removed = store
        .finish(&name("commits"), &removal.token, Finish::Removed)
        .await?;

    assert_eq!(removed, Owning::Held);
    assert!(store.get(&name("commits")).await?.is_none());
    assert!(store.list().await?.is_empty());

    Ok(())
}

/// Asking to create a name a dataset already answers to is a replacement, and
/// the answer says so rather than handing out an operation.
#[tokio::test]
async fn a_create_over_a_dataset_that_stands_answers_that_it_stands() -> R {
    let store = MemoryDatasets::at(at(0));
    let created = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();
    store
        .finish(
            &name("commits"),
            &created.token,
            Finish::Provisioned("ds_commits_1".to_owned()),
        )
        .await?;
    store
        .finish(&name("commits"), &created.token, Finish::Ready)
        .await?;

    let again = store.take_create(&name("commits"), &declaration()).await?;

    assert!(matches!(again, Taken::Stands), "{again:?}");
    let Some(held) = store.get(&name("commits")).await? else {
        panic!("the dataset stands");
    };
    assert_eq!(held.state, DatasetState::Ready, "it was never demoted");
    assert_eq!(held.physical_table, Some("ds_commits_1".to_owned()));

    Ok(())
}

#[tokio::test]
async fn replacing_a_dataset_nothing_stands_under_writes_nothing() -> R {
    let store = MemoryDatasets::at(at(0));

    assert!(
        !store.replace(&name("commits"), &declaration()).await?,
        "there is no dataset to replace"
    );

    let claimed = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();
    assert!(
        !store.replace(&name("commits"), &declaration()).await?,
        "one still being made does not stand either"
    );
    store
        .finish(&name("commits"), &claimed.token, Finish::Ready)
        .await?;

    assert!(store.replace(&name("commits"), &declaration()).await?);

    Ok(())
}

/// A removal that could not take the table away leaves the row mid-removal:
/// the name stays held, nothing reads it, and the lapsing lease is what lets
/// the request be repeated.
#[tokio::test]
async fn a_removal_that_could_not_finish_holds_the_name_until_its_lease_lapses() -> R {
    let store = MemoryDatasets::at(at(0));
    let created = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();
    store
        .finish(
            &name("commits"),
            &created.token,
            Finish::Provisioned("ds_commits_1".to_owned()),
        )
        .await?;
    store
        .finish(&name("commits"), &created.token, Finish::Ready)
        .await?;
    store.take_remove(&name("commits")).await?;

    // The drop failed, so nothing was finished. Nobody may take the name.
    let refused = store.take_create(&name("commits"), &declaration()).await;
    assert!(
        matches!(refused, Err(DatasetStoreError::Refused(Refused::Busy(_)))),
        "{refused:?}"
    );
    let Some(held) = store.get(&name("commits")).await? else {
        panic!("the row is still there");
    };
    assert_eq!(held.state, DatasetState::Removing);

    // Once the lease lapses the removal is repeatable, and completes.
    store.set_now(at(LEASE_SECS + 1));
    let again = store.take_remove(&name("commits")).await?;
    store
        .finish(&name("commits"), &again.token, Finish::Removed)
        .await?;

    assert!(store.get(&name("commits")).await?.is_none());

    Ok(())
}

/// A removal carries away the table the row named when it took the operation.
/// Reading the name when the drop runs would name whatever the row says then,
/// which after a take-over and a fresh create is somebody else's records.
#[tokio::test]
async fn a_removal_remembers_the_table_it_took_the_dataset_for() -> R {
    let store = MemoryDatasets::at(at(0));
    let first = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();
    store
        .finish(
            &name("commits"),
            &first.token,
            Finish::Provisioned("ds_commits_1".to_owned()),
        )
        .await?;
    store
        .finish(&name("commits"), &first.token, Finish::Ready)
        .await?;

    let stalled = store.take_remove(&name("commits")).await?;
    assert_eq!(stalled.table.as_deref(), Some("ds_commits_1"));

    // Its lease lapses, another removal completes, and the name is taken again
    // by a dataset whose records live somewhere else entirely.
    store.set_now(at(LEASE_SECS + 1));
    let repeated = store.take_remove(&name("commits")).await?;
    store
        .finish(&name("commits"), &repeated.token, Finish::Removed)
        .await?;
    let remade = store
        .take_create(&name("commits"), &declaration())
        .await?
        .attempt();
    store
        .finish(
            &name("commits"),
            &remade.token,
            Finish::Provisioned("ds_commits_2".to_owned()),
        )
        .await?;
    store
        .finish(&name("commits"), &remade.token, Finish::Ready)
        .await?;

    // The stalled removal still names what it took, not what stands now.
    assert_eq!(stalled.table.as_deref(), Some("ds_commits_1"));
    let Some(standing) = store.get(&name("commits")).await? else {
        panic!("the remade dataset stands");
    };
    assert_eq!(standing.physical_table.as_deref(), Some("ds_commits_2"));

    Ok(())
}
