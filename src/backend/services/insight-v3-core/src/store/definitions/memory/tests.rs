use std::error::Error;

use serde_json::json;

use super::*;

type R = Result<(), Box<dyn Error>>;

fn name(value: &str) -> DefinitionName {
    DefinitionName::parse(value).unwrap_or_else(|error| panic!("the name must parse: {error}"))
}

#[tokio::test]
async fn a_batch_refused_partway_leaves_the_store_as_it_was() -> R {
    let store = MemoryDefinitions::new();
    store
        .put(DefinitionKind::Metric, &name("taken"), &json!({ "was": 1 }))
        .await?;

    let refusal = store
        .apply(&[
            Change::Put(DefinitionKind::Metric, name("fresh"), json!({ "new": 1 })),
            Change::Create(DefinitionKind::Metric, name("taken"), json!({ "now": 2 })),
        ])
        .await;

    assert!(
        matches!(refusal, Err(DefinitionStoreError::NameTaken(_))),
        "{refusal:?}"
    );
    assert_eq!(
        store.get(DefinitionKind::Metric, &name("fresh")).await?,
        None,
        "the write before the refusal must not survive it"
    );
    assert_eq!(
        store.get(DefinitionKind::Metric, &name("taken")).await?,
        Some(json!({ "was": 1 }))
    );

    Ok(())
}

#[tokio::test]
async fn a_name_freed_earlier_in_the_batch_may_be_created_again() -> R {
    let store = MemoryDefinitions::new();
    store
        .put(DefinitionKind::Metric, &name("held"), &json!({ "was": 1 }))
        .await?;

    store
        .apply(&[
            Change::Delete(DefinitionKind::Metric, name("held")),
            Change::Create(DefinitionKind::Metric, name("held"), json!({ "now": 2 })),
        ])
        .await?;

    assert_eq!(
        store.get(DefinitionKind::Metric, &name("held")).await?,
        Some(json!({ "now": 2 }))
    );

    Ok(())
}
