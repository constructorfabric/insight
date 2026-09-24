use std::error::Error;

use serde_json::json;

use super::*;
use crate::domain::folders::{FolderError, FolderFilter, FolderName, Folders};

type R = Result<(), Box<dyn Error>>;

fn name(value: &str) -> DefinitionName {
    DefinitionName::parse(value).unwrap_or_else(|error| panic!("the name must parse: {error}"))
}

fn folder_name(value: &str) -> FolderName {
    FolderName::parse(value).unwrap_or_else(|error| panic!("the folder name must parse: {error}"))
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
        .put(
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

    assert_eq!(filed.names, vec!["delivery", "support"]);
    assert_eq!(filed.total, 2);
    assert_eq!(searched.names, vec!["delivery"]);
    assert_eq!(unfiled.names, vec!["delivery_ai"]);

    Ok(())
}

#[tokio::test]
async fn a_carried_folder_moves_with_the_name_in_the_same_batch() -> R {
    let store = with_boards(&["old"]).await;
    let platform = store.create_folder(folder_name("Platform")).await?;
    store.file(&name("old"), Some(platform.id)).await?;

    store
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                name("new"),
                json!({ "title": "old" }),
            ),
            Change::CarryFolder {
                from: name("old"),
                to: name("new"),
            },
            Change::Delete(DefinitionKind::Dashboard, name("old")),
        ])
        .await?;

    assert_eq!(
        store.folder_of(&name("new")).await?.map(|f| f.id),
        Some(platform.id)
    );
    assert_eq!(
        counts(&store.list_folders().await?),
        vec![("Platform".to_owned(), 1)]
    );

    Ok(())
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
