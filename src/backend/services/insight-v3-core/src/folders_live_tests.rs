use serde_json::json;
use uuid::Uuid;

use crate::domain::definition::{Change, DefinitionKind, DefinitionName, Definitions, Page};
use crate::domain::folders::{FolderError, FolderFilter, FolderName, Folders};
use crate::live_mariadb::{store_or_skip, unique};

fn dashboard(name: &str) -> DefinitionName {
    DefinitionName::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

fn folder_name(name: &str) -> FolderName {
    FolderName::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

#[tokio::test]
async fn a_folder_keeps_its_dashboards_through_writes_renames_and_its_own_deletion() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let board = dashboard(&unique("board"));
    let renamed = dashboard(&unique("renamed"));
    let label = unique("Platform");
    store
        .put(
            DefinitionKind::Dashboard,
            &board,
            &json!({ "title": "Board", "items": [] }),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let platform = store
        .create_folder(folder_name(&label))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let clash = store
        .create_folder(folder_name(&label.to_uppercase()))
        .await;
    assert!(matches!(clash, Err(FolderError::NameTaken(_))), "{clash:?}");

    store
        .file(&board, Some(platform.id))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    store
        .put(
            DefinitionKind::Dashboard,
            &board,
            &json!({ "title": "Rewritten", "items": [] }),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        store.folder_of(&board).await.ok().flatten().map(|f| f.id),
        Some(platform.id),
        "a body write must leave the folder alone"
    );

    store
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                renamed.clone(),
                json!({ "title": "Rewritten", "items": [] }),
            ),
            Change::CarryFolder {
                from: board.clone(),
                to: renamed.clone(),
            },
            Change::Delete(DefinitionKind::Dashboard, board.clone()),
        ])
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        store.folder_of(&renamed).await.ok().flatten().map(|f| f.id),
        Some(platform.id),
        "a rename must carry the folder"
    );

    let page = Page::parse(None, None).unwrap_or_else(|error| panic!("{error}"));
    let filed = store
        .page_filed("", page, FolderFilter::In(platform.id))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(filed.names, vec![renamed.as_str().to_owned()]);
    let listed = store
        .list_folders()
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let counted = listed
        .folders
        .iter()
        .find(|summary| summary.folder.id == platform.id)
        .map(|summary| summary.dashboards);
    assert_eq!(counted, Some(1));

    assert!(
        store
            .delete_folder(platform.id)
            .await
            .unwrap_or_else(|error| panic!("{error}"))
    );
    assert_eq!(store.folder_of(&renamed).await.ok().flatten(), None);
    let into_the_gone = store.file(&renamed, Some(platform.id)).await;
    assert!(
        matches!(into_the_gone, Err(FolderError::FolderNotFound(_))),
        "{into_the_gone:?}"
    );
    let nowhere = store.file(&dashboard(&unique("nowhere")), None).await;
    assert!(
        matches!(nowhere, Err(FolderError::DashboardNotFound(_))),
        "{nowhere:?}"
    );

    Definitions::delete(&store, DefinitionKind::Dashboard, &renamed)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

#[tokio::test]
async fn a_folder_renamed_to_its_own_name_in_another_case_keeps_its_id() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let label = unique("product");
    let product = store
        .create_folder(folder_name(&label))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let renamed = store
        .rename_folder(product.id, folder_name(&label.to_uppercase()))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(renamed.id, product.id);
    store
        .delete_folder(product.id)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

#[tokio::test]
async fn names_differing_by_accent_or_emoji_are_different_folders() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let tag = Uuid::now_v7().simple().to_string();
    let pairs = [
        (format!("Café {tag}"), format!("Cafe {tag}")),
        (format!("🚀 {tag}"), format!("🔥 {tag}")),
    ];

    for (first, second) in pairs {
        let one = store
            .create_folder(folder_name(&first))
            .await
            .unwrap_or_else(|error| panic!("{first}: {error}"));
        let other = store.create_folder(folder_name(&second)).await;

        assert!(other.is_ok(), "{second} beside {first}: {other:?}");
        for made in [Some(one), other.ok()].into_iter().flatten() {
            store
                .delete_folder(made.id)
                .await
                .unwrap_or_else(|error| panic!("{error}"));
        }
    }
}
