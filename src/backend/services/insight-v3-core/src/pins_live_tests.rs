use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;

use crate::domain::definition::{DefinitionKind, DefinitionName, Definitions};
use crate::domain::pins::{MAX_PINS, PinError, Pins};
use crate::domain::query::metric_query::People;
use crate::domain::surfaces::Surfaces;
use crate::live_mariadb::{store_or_skip, unique};
use crate::store::datasets::memory::MemoryDatasets;
use crate::store::definitions::MariaDefinitions;

fn dashboard(name: &str) -> DefinitionName {
    DefinitionName::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

fn boards(prefix: &str, count: usize) -> Vec<DefinitionName> {
    (0..count)
        .map(|index| dashboard(&unique(&format!("{prefix}{index}"))))
        .collect()
}

async fn stored(store: &MariaDefinitions, names: &[DefinitionName]) {
    for name in names {
        store
            .put(
                DefinitionKind::Dashboard,
                name,
                &json!({ "title": "Board", "items": [] }),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

async fn removed(store: &MariaDefinitions, names: &[DefinitionName]) {
    for name in names {
        Definitions::delete(store, DefinitionKind::Dashboard, name)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

async fn pin(store: &MariaDefinitions, person: Uuid, name: &DefinitionName) {
    store
        .pin(person, name)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

async fn pinned(store: &MariaDefinitions, person: Uuid) -> Vec<String> {
    store
        .pins_of(person)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
}

fn spelled(names: &[DefinitionName]) -> Vec<String> {
    names.iter().map(|name| name.as_str().to_owned()).collect()
}

#[tokio::test]
async fn each_person_sees_their_own_pins_oldest_first() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (anna, boris) = (Uuid::now_v7(), Uuid::now_v7());
    let named = boards("own", 3);
    stored(&store, &named).await;

    for name in [&named[2], &named[0], &named[1]] {
        pin(&store, anna, name).await;
    }
    pin(&store, boris, &named[1]).await;
    let annas = pinned(&store, anna).await;
    let boriss = pinned(&store, boris).await;

    removed(&store, &named).await;
    assert_eq!(
        annas,
        spelled(&[named[2].clone(), named[0].clone(), named[1].clone()])
    );
    assert_eq!(boriss, spelled(&named[1..2]));
}

#[tokio::test]
async fn a_repin_keeps_its_place_and_an_unpin_takes_only_that_one_off() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (anna, boris) = (Uuid::now_v7(), Uuid::now_v7());
    let named = boards("repin", 3);
    stored(&store, &named).await;
    for person in [anna, boris] {
        for name in &named {
            pin(&store, person, name).await;
        }
    }

    pin(&store, anna, &named[0]).await;
    store
        .unpin(anna, &named[1])
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let unpinned_twice = store.unpin(anna, &named[1]).await;
    let annas = pinned(&store, anna).await;
    let boriss = pinned(&store, boris).await;

    removed(&store, &named).await;
    assert!(unpinned_twice.is_ok(), "{unpinned_twice:?}");
    assert_eq!(annas, spelled(&[named[0].clone(), named[2].clone()]));
    assert_eq!(boriss, spelled(&named));
}

#[tokio::test]
async fn a_person_pins_twenty_dashboards_and_no_more() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (anna, boris) = (Uuid::now_v7(), Uuid::now_v7());
    let named = boards("cap", MAX_PINS + 1);
    stored(&store, &named).await;
    for name in &named[..MAX_PINS] {
        pin(&store, anna, name).await;
    }

    let refused = store.pin(anna, &named[MAX_PINS]).await;
    let repinned = store.pin(anna, &named[0]).await;
    let someone_else = store.pin(boris, &named[MAX_PINS]).await;
    let annas = pinned(&store, anna).await;

    removed(&store, &named).await;
    assert!(matches!(refused, Err(PinError::TooMany)), "{refused:?}");
    assert!(repinned.is_ok(), "{repinned:?}");
    assert!(someone_else.is_ok(), "{someone_else:?}");
    assert_eq!(annas, spelled(&named[..MAX_PINS]));
}

#[tokio::test]
async fn pinning_or_unpinning_a_dashboard_that_is_not_there_is_refused() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let nowhere = dashboard(&unique("nowhere"));

    let pinned_nothing = store.pin(Uuid::now_v7(), &nowhere).await;
    let unpinned_nothing = store.unpin(Uuid::now_v7(), &nowhere).await;

    for refused in [pinned_nothing, unpinned_nothing] {
        assert!(
            matches!(&refused, Err(PinError::DashboardNotFound(named)) if named == nowhere.as_str()),
            "{refused:?}"
        );
    }
}

#[tokio::test]
async fn a_rename_carries_every_pin_in_place_and_a_delete_drops_them() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (anna, boris) = (Uuid::now_v7(), Uuid::now_v7());
    let named = boards("carried", 3);
    let renamed = dashboard(&unique("renamed"));
    stored(&store, &named).await;
    for person in [anna, boris] {
        for name in &named {
            pin(&store, person, name).await;
        }
    }
    let people = People::new("identity");
    let datasets = MemoryDatasets::at(chrono::Utc::now());
    let surfaces = Surfaces::new(&store, &datasets, "insight_datasets", &people);

    surfaces
        .rename(DefinitionKind::Dashboard, &named[1], &renamed)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let carried = [pinned(&store, anna).await, pinned(&store, boris).await];
    removed(&store, &[named[0].clone(), renamed.clone()]).await;
    let left = [pinned(&store, anna).await, pinned(&store, boris).await];

    removed(&store, &named[2..]).await;
    for held in carried {
        assert_eq!(
            held,
            spelled(&[named[0].clone(), renamed.clone(), named[2].clone()])
        );
    }
    for held in left {
        assert_eq!(held, spelled(&named[2..]));
    }
}

#[tokio::test]
async fn pins_racing_for_the_last_places_never_pass_the_cap() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let store = Arc::new(store);
    let anna = Uuid::now_v7();
    let held = boards("held", MAX_PINS - 3);
    let racing = boards("racing", 8);
    stored(&store, &held).await;
    stored(&store, &racing).await;
    for name in &held {
        pin(&store, anna, name).await;
    }

    let writers: Vec<_> = racing
        .iter()
        .map(|name| {
            let store = Arc::clone(&store);
            let name = name.clone();
            tokio::spawn(async move { store.pin(anna, &name).await })
        })
        .collect();
    let mut landed = 0;
    let mut refused = 0;
    let mut failures = Vec::new();
    for writer in writers {
        match writer.await.unwrap_or_else(|error| panic!("{error}")) {
            Ok(()) => landed += 1,
            Err(PinError::TooMany) => refused += 1,
            Err(other) => failures.push(format!("{other:?}")),
        }
    }
    let total = pinned(&store, anna).await.len();

    removed(&store, &held).await;
    removed(&store, &racing).await;
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!((landed, refused), (3, 5));
    assert_eq!(total, MAX_PINS);
}
