use std::error::Error;

use serde_json::json;
use uuid::Uuid;

use crate::domain::access::{Caller, PersonId};
use crate::domain::definition::{
    Change, DefinitionId, DefinitionKind, DefinitionLabel, Definitions, Listed,
};
use crate::domain::pins::{self, MAX_PINS, PinError, Pins};
use crate::store::definitions::memory::MemoryDefinitions;

type R = Result<(), Box<dyn Error>>;

const ANNA: Caller = Caller::administrator(PersonId::new(Uuid::from_u128(1)));
const BORIS: Caller = Caller::administrator(PersonId::new(Uuid::from_u128(2)));

fn name(value: &str) -> DefinitionId {
    DefinitionId::parse(value).unwrap_or_else(|error| panic!("the name must parse: {error}"))
}

async fn with_boards(names: &[&str]) -> MemoryDefinitions {
    let store = MemoryDefinitions::new();
    for board in names {
        store
            .seed(
                DefinitionKind::Dashboard,
                &name(board),
                &json!({ "title": board }),
            )
            .await
            .unwrap_or_else(|error| panic!("`{board}` must store: {error}"));
    }
    store
}

async fn pinned(store: &MemoryDefinitions, person: Caller) -> Vec<String> {
    store
        .pins_of(person)
        .await
        .unwrap_or_else(|error| panic!("the pins must read: {error}"))
        .into_iter()
        .map(|listed| listed.id)
        .collect()
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
    let unpinned_nothing = pins::unpin(&store, ANNA, &name("nowhere")).await;

    for refused in [pinned_nothing, unpinned_nothing] {
        assert!(
            matches!(&refused, Err(PinError::DashboardNotFound(named)) if named == "nowhere"),
            "{refused:?}"
        );
    }
}

#[tokio::test]
async fn a_pin_answers_the_dashboard_s_name_beside_its_id() -> R {
    let store = MemoryDefinitions::new();
    store
        .create(
            DefinitionKind::Dashboard,
            &name("board-1"),
            &DefinitionLabel::parse("Delivery")?,
            &json!({ "title": "Delivery" }),
        )
        .await?;

    store.pin(ANNA, &name("board-1")).await?;

    let pins = store.pins_of(ANNA).await?;
    assert_eq!(
        pins,
        [Listed {
            id: "board-1".to_owned(),
            name: "Delivery".to_owned(),
        }]
    );
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

#[tokio::test]
async fn a_pin_of_a_dashboard_the_person_no_longer_sees_is_neither_listed_nor_counted() -> R {
    let boards: Vec<String> = (0..=MAX_PINS).map(|index| format!("b{index}")).collect();
    let named: Vec<&str> = boards.iter().map(String::as_str).collect();
    let store = with_boards(&named).await;
    for board in &boards[..MAX_PINS] {
        store.pin(ANNA, &name(board)).await?;
    }

    store
        .apply(&[Change::Own {
            dashboard: name(&boards[0]),
            owner: BORIS.person,
        }])
        .await?;
    let room = store.pin(ANNA, &name(&boards[MAX_PINS])).await;

    assert!(room.is_ok(), "{room:?}");
    assert!(!pinned(&store, ANNA).await.contains(&boards[0]));
    Ok(())
}

#[tokio::test]
async fn a_withdrawn_viewer_can_neither_pin_nor_file_the_dashboard() -> R {
    use crate::domain::access::{Entry, Grantee, Level};
    use crate::domain::folders::{FolderError, FolderName, Folders as _};
    use crate::domain::sharing::{Grants as _, Shared};

    let store = MemoryDefinitions::new();
    let board = name("delivery");
    store
        .seed_owned(&board, &json!({ "title": "delivery" }), ANNA.person)
        .await?;
    let folder = store
        .create_folder(BORIS.person, FolderName::parse("Mine")?)
        .await?;
    let shared = Shared::Dashboard(&board);
    store
        .grant(
            shared,
            Grantee::Person(BORIS.person),
            Level::Viewer,
            ANNA.person,
        )
        .await?;
    store
        .withdraw(shared, Grantee::Person(BORIS.person))
        .await?;

    let pinned_after = store.pin(BORIS, &board).await;
    let filed_after = store
        .add_entry(folder.id, &board, BORIS, Entry::Filing)
        .await;

    assert!(
        matches!(pinned_after, Err(PinError::DashboardNotFound(_))),
        "{pinned_after:?}"
    );
    assert!(
        matches!(filed_after, Err(FolderError::DashboardNotFound(_))),
        "{filed_after:?}"
    );
    assert_eq!(pinned(&store, BORIS).await, Vec::<String>::new());
    assert!(store.folders_holding(&board, BORIS).await?.is_empty());
    Ok(())
}
