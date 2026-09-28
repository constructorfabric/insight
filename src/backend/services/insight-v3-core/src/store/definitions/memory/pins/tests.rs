use std::error::Error;

use serde_json::json;
use uuid::Uuid;

use crate::domain::definition::{Change, DefinitionKind, DefinitionName, Definitions};
use crate::domain::pins::{MAX_PINS, PinError, Pins};
use crate::store::definitions::memory::MemoryDefinitions;

type R = Result<(), Box<dyn Error>>;

const ANNA: Uuid = Uuid::from_u128(1);
const BORIS: Uuid = Uuid::from_u128(2);

fn name(value: &str) -> DefinitionName {
    DefinitionName::parse(value).unwrap_or_else(|error| panic!("the name must parse: {error}"))
}

async fn with_boards(names: &[&str]) -> MemoryDefinitions {
    let store = MemoryDefinitions::new();
    for board in names {
        store
            .put(
                DefinitionKind::Dashboard,
                &name(board),
                &json!({ "title": board }),
            )
            .await
            .unwrap_or_else(|error| panic!("`{board}` must store: {error}"));
    }
    store
}

async fn pinned(store: &MemoryDefinitions, person: Uuid) -> Vec<String> {
    store
        .pins_of(person)
        .await
        .unwrap_or_else(|error| panic!("the pins must read: {error}"))
}

#[tokio::test]
async fn each_person_sees_only_their_own_pins() -> R {
    let store = with_boards(&["delivery", "hiring"]).await;

    store.pin(ANNA, &name("delivery")).await?;
    store.pin(BORIS, &name("hiring")).await?;

    assert_eq!(pinned(&store, ANNA).await, ["delivery"]);
    assert_eq!(pinned(&store, BORIS).await, ["hiring"]);
    Ok(())
}

#[tokio::test]
async fn pins_list_oldest_first() -> R {
    let store = with_boards(&["alpha", "beta", "gamma"]).await;

    for board in ["gamma", "alpha", "beta"] {
        store.pin(ANNA, &name(board)).await?;
    }

    assert_eq!(pinned(&store, ANNA).await, ["gamma", "alpha", "beta"]);
    Ok(())
}

#[tokio::test]
async fn pinning_a_pinned_dashboard_keeps_its_place() -> R {
    let store = with_boards(&["alpha", "beta"]).await;
    store.pin(ANNA, &name("alpha")).await?;
    store.pin(ANNA, &name("beta")).await?;

    store.pin(ANNA, &name("alpha")).await?;

    assert_eq!(pinned(&store, ANNA).await, ["alpha", "beta"]);
    Ok(())
}

#[tokio::test]
async fn unpinning_takes_the_dashboard_off_that_person_s_list_only() -> R {
    let store = with_boards(&["alpha", "beta"]).await;
    for person in [ANNA, BORIS] {
        store.pin(person, &name("alpha")).await?;
        store.pin(person, &name("beta")).await?;
    }

    store.unpin(ANNA, &name("alpha")).await?;

    assert_eq!(pinned(&store, ANNA).await, ["beta"]);
    assert_eq!(pinned(&store, BORIS).await, ["alpha", "beta"]);
    Ok(())
}

#[tokio::test]
async fn unpinning_a_dashboard_that_was_not_pinned_changes_nothing() -> R {
    let store = with_boards(&["alpha", "beta"]).await;
    store.pin(ANNA, &name("alpha")).await?;

    store.unpin(ANNA, &name("beta")).await?;

    assert_eq!(pinned(&store, ANNA).await, ["alpha"]);
    Ok(())
}

#[tokio::test]
async fn a_person_pins_twenty_dashboards_and_no_more() -> R {
    let boards: Vec<String> = (0..=MAX_PINS).map(|index| format!("b{index}")).collect();
    let named: Vec<&str> = boards.iter().map(String::as_str).collect();
    let store = with_boards(&named).await;
    for board in &boards[..MAX_PINS] {
        store.pin(ANNA, &name(board)).await?;
    }

    let refused = store.pin(ANNA, &name(&boards[MAX_PINS])).await;
    let repinned = store.pin(ANNA, &name(&boards[0])).await;
    let someone_else = store.pin(BORIS, &name(&boards[MAX_PINS])).await;

    assert!(matches!(refused, Err(PinError::TooMany)), "{refused:?}");
    assert!(repinned.is_ok(), "{repinned:?}");
    assert!(someone_else.is_ok(), "{someone_else:?}");
    assert_eq!(pinned(&store, ANNA).await, boards[..MAX_PINS]);
    Ok(())
}

#[tokio::test]
async fn pinning_or_unpinning_a_dashboard_that_is_not_there_is_refused() {
    let store = with_boards(&["alpha"]).await;

    let pinned_nothing = store.pin(ANNA, &name("nowhere")).await;
    let unpinned_nothing = store.unpin(ANNA, &name("nowhere")).await;

    for refused in [pinned_nothing, unpinned_nothing] {
        assert!(
            matches!(&refused, Err(PinError::DashboardNotFound(named)) if named == "nowhere"),
            "{refused:?}"
        );
    }
}

#[tokio::test]
async fn a_carried_pin_takes_the_old_name_s_place() -> R {
    let store = with_boards(&["alpha", "beta", "gamma"]).await;
    for person in [ANNA, BORIS] {
        for board in ["alpha", "beta", "gamma"] {
            store.pin(person, &name(board)).await?;
        }
    }

    store
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                name("renamed"),
                json!({ "title": "beta" }),
            ),
            Change::CarryPins {
                from: name("beta"),
                to: name("renamed"),
            },
            Change::Delete(DefinitionKind::Dashboard, name("beta")),
        ])
        .await?;

    for person in [ANNA, BORIS] {
        assert_eq!(pinned(&store, person).await, ["alpha", "renamed", "gamma"]);
    }
    Ok(())
}

#[tokio::test]
async fn a_deleted_dashboard_leaves_every_pinned_list() -> R {
    let store = with_boards(&["alpha", "beta"]).await;
    for person in [ANNA, BORIS] {
        store.pin(person, &name("alpha")).await?;
        store.pin(person, &name("beta")).await?;
    }

    Definitions::delete(&store, DefinitionKind::Dashboard, &name("alpha")).await?;

    for person in [ANNA, BORIS] {
        assert_eq!(pinned(&store, person).await, ["beta"]);
    }
    Ok(())
}

#[tokio::test]
async fn a_store_that_is_down_refuses_a_pin() {
    let store = MemoryDefinitions::refusing();

    let refused = store.pin(ANNA, &name("alpha")).await;

    assert!(matches!(refused, Err(PinError::Store(_))), "{refused:?}");
}
