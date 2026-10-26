use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::access::{Caller, Owner, PersonId};
use crate::domain::dashboards::{DashboardCatalogue as _, DashboardQuery};
use crate::domain::definition::{
    DefinitionId, DefinitionKind, DefinitionLabel, DefinitionStoreError, Definitions, Lookup,
};
use crate::domain::folders::{self, FolderName, Folders};
use crate::domain::pins::{self, Pins};
use crate::domain::query::metric_query::People;
use crate::domain::surfaces::Surfaces;
use crate::domain::tags::{self, TagSet, Tags};
use crate::live_mariadb::{store_or_skip, unique};
use crate::store::datasets::memory::MemoryDatasets;
use crate::store::definitions::MariaDefinitions;
use crate::tags_live_tests::ONE_AT_A_TIME;

fn dashboard(name: &str) -> DefinitionId {
    DefinitionId::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

fn body(title: &str) -> Value {
    json!({ "title": title, "items": [] })
}

fn a_tag() -> String {
    let id = Uuid::now_v7().simple().to_string();

    format!("Delivery {}", &id[id.len() - 12..])
}

async fn stored(store: &MariaDefinitions, name: &DefinitionId, title: &str) {
    store
        .create(
            DefinitionKind::Dashboard,
            name,
            &DefinitionLabel::of_id(name),
            &body(title),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

async fn removed(store: &MariaDefinitions, names: &[&DefinitionId]) {
    for name in names {
        Definitions::delete(store, DefinitionKind::Dashboard, name)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

#[derive(Debug, PartialEq)]
struct Held {
    body: Option<Value>,
    folders: Vec<String>,
    tags: Vec<String>,
}

async fn held(store: &MariaDefinitions, name: &DefinitionId, person: Caller) -> Held {
    let body = store
        .get(DefinitionKind::Dashboard, name)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let folders = store
        .folders_holding(name, person)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .map(|folder| folder.name.as_str().to_owned())
        .collect();
    let tags = store
        .tags_of(name)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .map(|tag| tag.as_str().to_owned())
        .collect();

    Held {
        body,
        folders,
        tags,
    }
}

#[tokio::test]
async fn a_copy_is_the_callers_with_the_body_their_folder_and_tags_and_no_pins() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let anna = Caller::administrator(PersonId::new(Uuid::now_v7()));
    let people = People::new("identity");
    let datasets = MemoryDatasets::at(chrono::Utc::now());
    let surfaces = Surfaces::new(&store, &datasets, "insight_datasets", &people, anna);
    let label = unique("Platform");
    let tag = a_tag();
    let source = surfaces
        .create(
            DefinitionKind::Dashboard,
            &DefinitionLabel::parse("Source").unwrap_or_else(|error| panic!("{error}")),
            &body("Source"),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let folder = store
        .create_folder(
            anna.person,
            FolderName::parse(&label).unwrap_or_else(|error| panic!("{error}")),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    folders::add(&store, anna, folder.id, &source)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    tags::retag(
        &store,
        anna,
        &source,
        &TagSet::parse(std::slice::from_ref(&tag)).unwrap_or_else(|error| panic!("{error}")),
    )
    .await
    .unwrap_or_else(|error| panic!("{error}"));
    pins::pin(&store, anna, &source)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let copy = surfaces
        .duplicate(
            &source,
            &DefinitionLabel::parse("Source copy").unwrap_or_else(|error| panic!("{error}")),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let named = store
        .read(DefinitionKind::Dashboard, &copy)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .map(|stored| stored.name);
    let owner = store
        .standing(&copy, anna)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .map(|standing| standing.owner);
    let copied = held(&store, &copy, anna).await;
    let kept = held(&store, &source, anna).await;
    let pins = store
        .pins_of(anna)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    removed(&store, &[&source, &copy]).await;
    store
        .delete_folder(folder.id)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let expected = Held {
        body: Some(body("Source")),
        folders: vec![label],
        tags: vec![tag],
    };
    assert_eq!(copied, expected);
    assert_eq!(kept, expected);
    assert_eq!(named.as_deref(), Some("Source copy"));
    assert_eq!(owner, Some(Owner::Person(anna.person)));
    let pins: Vec<&str> = pins.iter().map(|listed| listed.id.as_str()).collect();
    assert_eq!(pins, [source.as_str()]);
}

#[tokio::test]
async fn a_create_onto_an_id_already_held_is_refused_and_leaves_that_dashboard_as_it_was() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let taken = dashboard(&unique("taken"));
    stored(&store, &taken, "Taken").await;

    let refused = store
        .create(
            DefinitionKind::Dashboard,
            &taken,
            &DefinitionLabel::of_id(&taken),
            &body("Again"),
        )
        .await;
    let left = held(
        &store,
        &taken,
        Caller::administrator(PersonId::new(Uuid::now_v7())),
    )
    .await;

    removed(&store, &[&taken]).await;
    assert!(
        matches!(&refused, Err(DefinitionStoreError::IdTaken(id)) if id == taken.as_str()),
        "{refused:?}"
    );
    assert_eq!(
        left,
        Held {
            body: Some(body("Taken")),
            folders: Vec::new(),
            tags: Vec::new(),
        }
    );
}

#[tokio::test]
async fn two_dashboards_may_share_a_name_and_a_page_orders_them_by_name_then_id() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let shared = unique("Shared board");
    let label = DefinitionLabel::parse(&shared).unwrap_or_else(|error| panic!("{error}"));
    let mut ids = [dashboard(&unique("b")), dashboard(&unique("a"))];
    for id in &ids {
        store
            .create(DefinitionKind::Dashboard, id, &label, &body("Shared"))
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }

    let page = crate::domain::definition::Page::parse(None, None)
        .unwrap_or_else(|error| panic!("{error}"));
    let query = DashboardQuery {
        needle: shared.clone(),
        ..DashboardQuery::default()
    };
    let found = store
        .page_dashboards(
            Caller::administrator(PersonId::new(Uuid::now_v7())),
            &query,
            page,
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    removed(&store, &[&ids[0], &ids[1]]).await;
    ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    let listed: Vec<(&str, &str)> = found
        .items
        .iter()
        .map(|one| (one.id.as_str(), one.name.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            (ids[0].as_str(), shared.as_str()),
            (ids[1].as_str(), shared.as_str())
        ]
    );
    assert_eq!(found.total, 2);
}
