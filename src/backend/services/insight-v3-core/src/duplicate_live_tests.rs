use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::definition::{
    DefinitionKind, DefinitionName, DefinitionStoreError, Definitions, Lookup,
};
use crate::domain::folders::{FolderName, Folders};
use crate::domain::pins::Pins;
use crate::domain::query::metric_query::People;
use crate::domain::surfaces::{CustomError, Surfaces};
use crate::domain::tags::{TagSet, Tags};
use crate::live_mariadb::{store_or_skip, unique};
use crate::store::datasets::memory::MemoryDatasets;
use crate::store::definitions::MariaDefinitions;
use crate::tags_live_tests::ONE_AT_A_TIME;

fn dashboard(name: &str) -> DefinitionName {
    DefinitionName::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

fn body(title: &str) -> Value {
    json!({ "title": title, "items": [] })
}

fn a_tag() -> String {
    let id = Uuid::now_v7().simple().to_string();

    format!("Delivery {}", &id[id.len() - 12..])
}

async fn stored(store: &MariaDefinitions, name: &DefinitionName, title: &str) {
    store
        .put(DefinitionKind::Dashboard, name, &body(title))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

async fn removed(store: &MariaDefinitions, names: &[&DefinitionName]) {
    for name in names {
        Definitions::delete(store, DefinitionKind::Dashboard, name)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

#[derive(Debug, PartialEq)]
struct Held {
    body: Option<Value>,
    folder: Option<String>,
    tags: Vec<String>,
}

async fn held(store: &MariaDefinitions, name: &DefinitionName) -> Held {
    let body = store
        .get(DefinitionKind::Dashboard, name)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let folder = store
        .folder_of(name)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .map(|folder| folder.name.as_str().to_owned());
    let tags = store
        .tags_of(name)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .map(|tag| tag.as_str().to_owned())
        .collect();

    Held { body, folder, tags }
}

#[tokio::test]
async fn a_copy_holds_the_body_folder_and_tags_and_no_pins_while_the_source_keeps_all_four() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let anna = Uuid::now_v7();
    let source = dashboard(&unique("source"));
    let copy = dashboard(&unique("copy"));
    let label = unique("Platform");
    let tag = a_tag();
    stored(&store, &source, "Source").await;
    let folder = store
        .create_folder(FolderName::parse(&label).unwrap_or_else(|error| panic!("{error}")))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    store
        .file(&source, Some(folder.id))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    store
        .set_tags(
            &source,
            &TagSet::parse(std::slice::from_ref(&tag)).unwrap_or_else(|error| panic!("{error}")),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    store
        .pin(anna, &source)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let people = People::new("identity");
    let datasets = MemoryDatasets::at(chrono::Utc::now());
    let surfaces = Surfaces::new(&store, &datasets, "insight_datasets", &people);

    surfaces
        .duplicate(&source, &copy)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let copied = held(&store, &copy).await;
    let kept = held(&store, &source).await;
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
        folder: Some(label),
        tags: vec![tag],
    };
    assert_eq!(copied, expected);
    assert_eq!(kept, expected);
    assert_eq!(pins, [source.as_str()]);
}

#[tokio::test]
async fn a_copy_onto_a_name_already_held_is_refused_and_leaves_that_dashboard_as_it_was() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let source = dashboard(&unique("source"));
    let taken = dashboard(&unique("taken"));
    stored(&store, &source, "Source").await;
    stored(&store, &taken, "Taken").await;
    store
        .set_tags(
            &source,
            &TagSet::parse(&[a_tag()]).unwrap_or_else(|error| panic!("{error}")),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let people = People::new("identity");
    let datasets = MemoryDatasets::at(chrono::Utc::now());
    let surfaces = Surfaces::new(&store, &datasets, "insight_datasets", &people);

    let refused = surfaces.duplicate(&source, &taken).await;
    let left = held(&store, &taken).await;

    removed(&store, &[&source, &taken]).await;
    assert!(
        matches!(
            &refused,
            Err(CustomError::Store(DefinitionStoreError::NameTaken(name))) if name == taken.as_str()
        ),
        "{refused:?}"
    );
    assert_eq!(
        left,
        Held {
            body: Some(body("Taken")),
            folder: None,
            tags: Vec::new(),
        }
    );
}
