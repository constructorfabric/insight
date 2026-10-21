use std::error::Error;

use serde_json::json;

use super::*;
use crate::domain::folders::{FolderError, FolderFilter, FolderName, Folders};

type R = Result<(), Box<dyn Error>>;

fn name(value: &str) -> DefinitionId {
    DefinitionId::parse(value).unwrap_or_else(|error| panic!("the name must parse: {error}"))
}

fn folder_name(value: &str) -> FolderName {
    FolderName::parse(value).unwrap_or_else(|error| panic!("the folder name must parse: {error}"))
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

fn counts(list: &crate::domain::folders::FolderList) -> Vec<(String, u64)> {
    list.folders
        .iter()
        .map(|summary| (summary.folder.name.as_str().to_owned(), summary.dashboards))
        .collect()
}

#[tokio::test]
async fn a_new_folder_is_listed_empty_beside_the_unfiled_count() -> R {
    let store = with_boards(&["delivery", "support"]).await;

    store.create_folder(folder_name("Platform")).await?;
    let listed = store.list_folders().await?;

    assert_eq!(counts(&listed), vec![("Platform".to_owned(), 0)]);
    assert_eq!(listed.unfiled, 2);

    Ok(())
}

#[tokio::test]
async fn a_folder_name_taken_in_another_case_is_refused() -> R {
    let store = MemoryDefinitions::new();
    store.create_folder(folder_name("Platform")).await?;

    let clash = store.create_folder(folder_name("platform")).await;

    assert!(matches!(clash, Err(FolderError::NameTaken(_))), "{clash:?}");
    assert_eq!(store.list_folders().await?.folders.len(), 1);

    Ok(())
}

#[tokio::test]
async fn a_rename_onto_a_name_another_folder_holds_is_refused() -> R {
    let store = MemoryDefinitions::new();
    store.create_folder(folder_name("Platform")).await?;
    let product = store.create_folder(folder_name("Product")).await?;

    let clash = store
        .rename_folder(product.id, folder_name("PLATFORM"))
        .await;

    assert!(matches!(clash, Err(FolderError::NameTaken(_))), "{clash:?}");

    Ok(())
}

#[tokio::test]
async fn a_folder_may_be_renamed_to_its_own_name_in_another_case() -> R {
    let store = MemoryDefinitions::new();
    let platform = store.create_folder(folder_name("platform")).await?;

    let renamed = store
        .rename_folder(platform.id, folder_name("Platform"))
        .await?;

    assert_eq!(renamed.name.as_str(), "Platform");

    Ok(())
}

#[tokio::test]
async fn a_filed_dashboard_counts_under_its_folder_and_not_as_unfiled() -> R {
    let store = with_boards(&["delivery", "support"]).await;
    let platform = store.create_folder(folder_name("Platform")).await?;

    store.file(&name("delivery"), Some(platform.id)).await?;
    let listed = store.list_folders().await?;

    assert_eq!(counts(&listed), vec![("Platform".to_owned(), 1)]);
    assert_eq!(listed.unfiled, 1);
    assert_eq!(
        store.folder_of(&name("delivery")).await?.map(|f| f.id),
        Some(platform.id)
    );

    Ok(())
}

#[tokio::test]
async fn filing_into_a_folder_that_is_gone_leaves_the_dashboard_where_it_was() -> R {
    let store = with_boards(&["delivery"]).await;
    let platform = store.create_folder(folder_name("Platform")).await?;
    let gone = store.create_folder(folder_name("Gone")).await?;
    store.file(&name("delivery"), Some(platform.id)).await?;
    store.delete_folder(gone.id).await?;

    let refused = store.file(&name("delivery"), Some(gone.id)).await;

    assert!(
        matches!(refused, Err(FolderError::FolderNotFound(_))),
        "{refused:?}"
    );
    assert_eq!(
        store.folder_of(&name("delivery")).await?.map(|f| f.id),
        Some(platform.id)
    );

    Ok(())
}

#[tokio::test]
async fn filing_a_dashboard_that_does_not_exist_is_refused() -> R {
    let store = MemoryDefinitions::new();
    let platform = store.create_folder(folder_name("Platform")).await?;

    let refused = store.file(&name("nowhere"), Some(platform.id)).await;

    assert!(
        matches!(refused, Err(FolderError::DashboardNotFound(_))),
        "{refused:?}"
    );

    Ok(())
}

#[tokio::test]
async fn deleting_a_folder_leaves_its_dashboards_unfiled() -> R {
    let store = with_boards(&["delivery", "support"]).await;
    let platform = store.create_folder(folder_name("Platform")).await?;
    store.file(&name("delivery"), Some(platform.id)).await?;

    assert!(store.delete_folder(platform.id).await?);
    let listed = store.list_folders().await?;

    assert!(listed.folders.is_empty());
    assert_eq!(listed.unfiled, 2);
    assert_eq!(store.folder_of(&name("delivery")).await?, None);

    Ok(())
}

#[tokio::test]
async fn deleting_a_filed_dashboard_takes_it_out_of_its_folder_count() -> R {
    let store = with_boards(&["delivery"]).await;
    let platform = store.create_folder(folder_name("Platform")).await?;
    store.file(&name("delivery"), Some(platform.id)).await?;

    Definitions::delete(&store, DefinitionKind::Dashboard, &name("delivery")).await?;

    assert_eq!(
        counts(&store.list_folders().await?),
        vec![("Platform".to_owned(), 0)]
    );

    Ok(())
}

#[tokio::test]
async fn a_body_write_leaves_the_dashboard_in_its_folder() -> R {
    let store = with_boards(&["delivery"]).await;
    let platform = store.create_folder(folder_name("Platform")).await?;
    store.file(&name("delivery"), Some(platform.id)).await?;

    store
        .seed(
            DefinitionKind::Dashboard,
            &name("delivery"),
            &json!({ "title": "Rewritten" }),
        )
        .await?;

    assert_eq!(
        store.folder_of(&name("delivery")).await?.map(|f| f.id),
        Some(platform.id)
    );

    Ok(())
}

fn ids(page: &ListedPage) -> Vec<&str> {
    page.items.iter().map(|listed| listed.id.as_str()).collect()
}

fn label(value: &str) -> DefinitionLabel {
    DefinitionLabel::parse(value).unwrap_or_else(|error| panic!("the label must parse: {error}"))
}

async fn stamp(store: &MemoryDefinitions, id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    store
        .read(DefinitionKind::Dashboard, &name(id))
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .map(|stored| stored.updated_at)
}

#[tokio::test]
async fn a_page_of_one_folder_holds_only_its_dashboards_and_still_searches() -> R {
    let store = with_boards(&["delivery", "delivery_ai", "support"]).await;
    let platform = store.create_folder(folder_name("Platform")).await?;
    store.file(&name("delivery"), Some(platform.id)).await?;
    store.file(&name("support"), Some(platform.id)).await?;
    let page = Page::parse(None, None)?;

    let filed = store
        .page_filed("", page, FolderFilter::In(platform.id))
        .await?;
    let searched = store
        .page_filed("deliv", page, FolderFilter::In(platform.id))
        .await?;
    let unfiled = store.page_filed("", page, FolderFilter::Unfiled).await?;

    assert_eq!(ids(&filed), vec!["delivery", "support"]);
    assert_eq!(filed.total, 2);
    assert_eq!(ids(&searched), vec!["delivery"]);
    assert_eq!(ids(&unfiled), vec!["delivery_ai"]);

    Ok(())
}

#[tokio::test]
async fn a_page_is_ordered_by_name_then_id_and_answers_both() -> R {
    let store = MemoryDefinitions::new();
    for (id, called) in [("b2", "Shared"), ("a1", "Zeta"), ("a9", "Shared")] {
        store
            .create(
                DefinitionKind::Widget,
                &name(id),
                &label(called),
                &json!({ "type": "table" }),
            )
            .await?;
    }

    let listed = store
        .page(DefinitionKind::Widget, "", Page::parse(None, None)?)
        .await?;

    let seen: Vec<(&str, &str)> = listed
        .items
        .iter()
        .map(|one| (one.id.as_str(), one.name.as_str()))
        .collect();
    assert_eq!(
        seen,
        vec![("a9", "Shared"), ("b2", "Shared"), ("a1", "Zeta")]
    );
    assert_eq!(listed.total, 3);

    Ok(())
}

#[tokio::test]
async fn a_search_matches_the_name_as_well_as_the_body() -> R {
    let store = MemoryDefinitions::new();
    store
        .create(
            DefinitionKind::Metric,
            &name("m1"),
            &label("Lead time"),
            &json!({ "table": "events" }),
        )
        .await?;

    let found = store
        .page(DefinitionKind::Metric, "lead", Page::parse(None, None)?)
        .await?;

    assert_eq!(ids(&found), vec!["m1"]);

    Ok(())
}

#[tokio::test]
async fn a_carried_folder_moves_to_the_copy_in_the_same_batch() -> R {
    let store = with_boards(&["original"]).await;
    let platform = store.create_folder(folder_name("Platform")).await?;
    store.file(&name("original"), Some(platform.id)).await?;

    store
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                name("copy"),
                label("Copy"),
                json!({ "title": "original" }),
            ),
            Change::CarryFolder {
                from: name("original"),
                to: name("copy"),
            },
        ])
        .await?;

    assert_eq!(
        store.folder_of(&name("copy")).await?.map(|f| f.id),
        Some(platform.id)
    );
    assert_eq!(
        counts(&store.list_folders().await?),
        vec![("Platform".to_owned(), 2)]
    );

    Ok(())
}

#[tokio::test]
async fn a_batch_refused_partway_leaves_the_store_as_it_was() -> R {
    let store = MemoryDefinitions::new();
    store
        .seed(DefinitionKind::Metric, &name("taken"), &json!({ "was": 1 }))
        .await?;

    let refusal = store
        .apply(&[
            Change::Create(
                DefinitionKind::Metric,
                name("fresh"),
                label("Fresh"),
                json!({ "new": 1 }),
            ),
            Change::Create(
                DefinitionKind::Metric,
                name("taken"),
                label("Taken"),
                json!({ "now": 2 }),
            ),
        ])
        .await;

    assert!(
        matches!(refusal, Err(DefinitionStoreError::IdTaken(_))),
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
async fn a_batch_replacing_a_missing_id_stores_none_of_it() -> R {
    let store = MemoryDefinitions::new();

    let refusal = store
        .apply(&[
            Change::Create(
                DefinitionKind::Metric,
                name("fresh"),
                label("Fresh"),
                json!({ "new": 1 }),
            ),
            Change::Replace(
                DefinitionKind::Widget,
                name("nowhere"),
                label("Nowhere"),
                json!({ "type": "table" }),
            ),
        ])
        .await;

    assert!(
        matches!(&refusal, Err(DefinitionStoreError::Missing(id)) if id == "nowhere"),
        "{refusal:?}"
    );
    assert_eq!(
        store.get(DefinitionKind::Metric, &name("fresh")).await?,
        None
    );

    Ok(())
}

#[tokio::test]
async fn a_replace_renames_and_rewrites_only_what_exists() -> R {
    let store = MemoryDefinitions::new();
    store
        .create(
            DefinitionKind::Metric,
            &name("held"),
            &label("Before"),
            &json!({ "was": 1 }),
        )
        .await?;

    let replaced = store
        .replace(
            DefinitionKind::Metric,
            &name("held"),
            &label("After"),
            &json!({ "now": 2 }),
        )
        .await?;
    let missing = store
        .replace(
            DefinitionKind::Metric,
            &name("never"),
            &label("After"),
            &json!({ "now": 2 }),
        )
        .await?;

    let stored = store.read(DefinitionKind::Metric, &name("held")).await?;
    assert!(replaced);
    assert!(!missing);
    assert_eq!(
        stored.map(|stored| (stored.name, stored.body)),
        Some(("After".to_owned(), json!({ "now": 2 })))
    );
    assert_eq!(
        store.get(DefinitionKind::Metric, &name("never")).await?,
        None
    );

    Ok(())
}

#[tokio::test]
async fn a_create_onto_a_held_id_is_refused() -> R {
    let store = MemoryDefinitions::new();
    store
        .seed(DefinitionKind::Metric, &name("held"), &json!({ "was": 1 }))
        .await?;

    let refused = store
        .create(
            DefinitionKind::Metric,
            &name("held"),
            &label("Again"),
            &json!({ "now": 2 }),
        )
        .await;

    assert!(
        matches!(refused, Err(DefinitionStoreError::IdTaken(_))),
        "{refused:?}"
    );

    Ok(())
}

#[tokio::test]
async fn a_folder_past_the_cap_is_refused() -> R {
    let store = MemoryDefinitions::new();
    for index in 0..crate::domain::folders::MAX_FOLDERS {
        store
            .create_folder(folder_name(&format!("f{index}")))
            .await?;
    }

    let refused = store.create_folder(folder_name("one more")).await;

    assert!(matches!(refused, Err(FolderError::TooMany)), "{refused:?}");

    Ok(())
}

#[tokio::test]
async fn a_write_is_stamped_when_it_lands_and_a_later_write_moves_the_stamp() -> R {
    let store = MemoryDefinitions::new();
    let before = chrono::Utc::now();

    store
        .seed(
            DefinitionKind::Dashboard,
            &name("board"),
            &json!({ "title": "A" }),
        )
        .await?;
    let first = stamp(&store, "board").await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    store
        .apply(&[Change::Replace(
            DefinitionKind::Dashboard,
            name("board"),
            label("Board"),
            json!({ "title": "B" }),
        )])
        .await?;
    let second = stamp(&store, "board").await;

    assert!(first.is_some_and(|stamp| stamp >= before), "{first:?}");
    assert!(second > first, "{first:?} then {second:?}");
    Ok(())
}

#[tokio::test]
async fn a_created_definition_is_stamped_and_a_removed_one_is_not() -> R {
    let store = MemoryDefinitions::new();

    store
        .apply(&[Change::Create(
            DefinitionKind::Dashboard,
            name("created"),
            label("Created"),
            json!({ "title": "events" }),
        )])
        .await?;
    let created = stamp(&store, "created").await;
    Definitions::delete(&store, DefinitionKind::Dashboard, &name("created")).await?;
    let removed = stamp(&store, "created").await;
    let never = stamp(&store, "never").await;

    assert!(created.is_some());
    assert_eq!((removed, never), (None, None));
    Ok(())
}
