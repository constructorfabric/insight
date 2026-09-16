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
        .take_operation(&name("commits"), Operation::Create, &declaration())
        .await?;

    assert_eq!(attempt.state, DatasetState::Claimed);
    let Some(held) = store.get(&name("commits")).await? else {
        panic!("the claim left a row");
    };
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
        .take_operation(&name("commits"), Operation::Create, &declaration())
        .await?;

    let refused = store
        .take_operation(&name("commits"), Operation::Create, &declaration())
        .await;

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
        .take_operation(&name("commits"), Operation::Create, &declaration())
        .await?;

    store.set_now(at(LEASE_SECS + 1));
    let second = store
        .take_operation(&name("commits"), Operation::Create, &declaration())
        .await?;

    assert_ne!(
        second.token, first.token,
        "the second attempt owns the dataset under its own token"
    );
    assert_ne!(
        second.token.table(),
        first.token.table(),
        "and provisions a table the first cannot drop"
    );

    Ok(())
}

#[tokio::test]
async fn a_dataset_that_was_never_there_has_nothing_to_remove() -> R {
    let store = MemoryDatasets::at(at(0));

    let refused = store
        .take_operation(&name("commits"), Operation::Remove, &declaration())
        .await;

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
        .take_operation(&name("commits"), Operation::Create, &declaration())
        .await?;
    store.set_now(at(LEASE_SECS + 1));
    store
        .take_operation(&name("commits"), Operation::Remove, &declaration())
        .await?;

    store.set_now(at(2 * LEASE_SECS + 2));
    let refused = store
        .take_operation(&name("commits"), Operation::Create, &declaration())
        .await;

    assert!(
        matches!(refused, Err(DatasetStoreError::Refused(Refused::Removing))),
        "{refused:?}"
    );

    Ok(())
}
