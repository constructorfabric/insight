use serde_json::json;
use uuid::Uuid;

use crate::domain::access::{Caller, Entry, Owner, PersonId};
use crate::domain::dashboards::{DashboardCatalogue as _, DashboardQuery};
use crate::domain::definition::{
    Change, DefinitionId, DefinitionKind, DefinitionLabel, Definitions, Page,
};
use crate::domain::folders::{FolderError, FolderFilter, FolderId, FolderName, Folders};
use crate::live_mariadb::{store_or_skip, unique};
use crate::store::definitions::MariaDefinitions;

fn dashboard(name: &str) -> DefinitionId {
    DefinitionId::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

fn folder_name(name: &str) -> FolderName {
    FolderName::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

fn a_person() -> PersonId {
    PersonId::new(Uuid::now_v7())
}

async fn created(store: &MariaDefinitions, id: &DefinitionId, owner: PersonId) {
    store
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                id.clone(),
                DefinitionLabel::of_id(id),
                json!({ "title": "Board", "items": [] }),
            ),
            Change::Own {
                dashboard: id.clone(),
                owner,
            },
        ])
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

/// A copy of `from` as `owner`'s, in the folders of the original they may
/// arrange.
async fn copied(store: &MariaDefinitions, from: &DefinitionId, to: &DefinitionId, owner: PersonId) {
    store
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                to.clone(),
                DefinitionLabel::of_id(to),
                json!({ "title": "Rewritten", "items": [] }),
            ),
            Change::Own {
                dashboard: to.clone(),
                owner,
            },
            Change::CarryEntries {
                from: from.clone(),
                to: to.clone(),
                placer: Caller::administrator(owner),
            },
        ])
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

/// The ids of the dashboards `person` sees in the folder.
async fn filed_in(store: &MariaDefinitions, person: PersonId, folder: FolderId) -> Vec<String> {
    let page = Page::parse(None, None).unwrap_or_else(|error| panic!("{error}"));
    let query = DashboardQuery {
        folder: Some(FolderFilter::In(folder)),
        ..DashboardQuery::default()
    };

    store
        .page_dashboards(Caller::administrator(person), &query, page)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .items
        .into_iter()
        .map(|listed| listed.id)
        .collect()
}

/// The folders `person` sees holding the dashboard.
async fn holding(
    store: &MariaDefinitions,
    board: &DefinitionId,
    person: PersonId,
) -> Vec<FolderId> {
    store
        .folders_holding(board, Caller::administrator(person))
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .map(|folder| folder.id)
        .collect()
}

#[tokio::test]
async fn a_folder_keeps_its_dashboards_through_writes_copies_and_its_own_deletion() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let me = a_person();
    let board = dashboard(&unique("board"));
    let copy = dashboard(&unique("copy"));
    let label = unique("Platform");
    created(&store, &board, me).await;

    let platform = store
        .create_folder(me, folder_name(&label))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let alike = store
        .create_folder(me, folder_name(&label.to_uppercase()))
        .await;
    assert!(alike.is_ok(), "two folders may share a name: {alike:?}");

    let as_me = Caller::administrator(me);
    store
        .add_entry(platform.id, &board, as_me, Entry::Carrying)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    store
        .add_entry(platform.id, &board, as_me, Entry::Filing)
        .await
        .unwrap_or_else(|error| panic!("adding an entry already there is the same entry: {error}"));
    let replaced = store
        .replace(
            DefinitionKind::Dashboard,
            &board,
            &DefinitionLabel::of_id(&board),
            &json!({ "title": "Rewritten", "items": [] }),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(replaced);
    assert_eq!(
        holding(&store, &board, me).await,
        vec![platform.id],
        "a body write must leave the folder alone"
    );

    copied(&store, &board, &copy, me).await;
    Definitions::delete(&store, DefinitionKind::Dashboard, &board)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        holding(&store, &copy, me).await,
        vec![platform.id],
        "a copy must carry the entry"
    );

    assert_eq!(filed_in(&store, me, platform.id).await, vec![copy.as_str()]);
    let listed = store
        .list_folders(as_me)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let counted = listed
        .folders
        .iter()
        .find(|summary| summary.seen.folder.id == platform.id)
        .map(|summary| summary.dashboards);
    assert_eq!(counted, Some(1));
    let elsewhere = store
        .list_folders(Caller::administrator(a_person()))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        elsewhere
            .folders
            .iter()
            .all(|summary| summary.seen.folder.id != platform.id),
        "another person does not see my folder"
    );

    assert!(
        store
            .delete_folder(platform.id)
            .await
            .unwrap_or_else(|error| panic!("{error}"))
    );
    assert!(holding(&store, &copy, me).await.is_empty());
    let into_the_gone = store
        .add_entry(platform.id, &copy, as_me, Entry::Carrying)
        .await;
    assert!(
        matches!(into_the_gone, Err(FolderError::FolderNotFound(_))),
        "{into_the_gone:?}"
    );

    Definitions::delete(&store, DefinitionKind::Dashboard, &copy)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    if let Ok(folder) = alike {
        store
            .delete_folder(folder.id)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

#[tokio::test]
async fn an_entry_of_a_dashboard_nobody_holds_is_refused() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let me = a_person();
    let folder = store
        .create_folder(me, folder_name(&unique("empty")))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let nowhere = store
        .add_entry(
            folder.id,
            &dashboard(&unique("nowhere")),
            Caller::administrator(me),
            Entry::Carrying,
        )
        .await;

    store
        .delete_folder(folder.id)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        matches!(nowhere, Err(FolderError::DashboardNotFound(_))),
        "{nowhere:?}"
    );
}

#[tokio::test]
async fn a_folder_renamed_to_its_own_name_in_another_case_keeps_its_id() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let me = a_person();
    let label = unique("product");
    let product = store
        .create_folder(me, folder_name(&label))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let renamed = store
        .rename_folder(product.id, &folder_name(&label.to_uppercase()))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let held = store
        .held_folder(product.id, Caller::administrator(me))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    store
        .delete_folder(product.id)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(renamed);
    let held = held.unwrap_or_else(|| panic!("the folder is there"));
    assert_eq!(held.folder.id, product.id);
    assert_eq!(held.folder.owner, Owner::Person(me));
    assert_eq!(held.folder.name.as_str(), label.to_uppercase());
}

#[tokio::test]
async fn names_differing_by_accent_or_emoji_are_different_folders() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let me = a_person();
    let tag = Uuid::now_v7().simple().to_string();
    let pairs = [
        (format!("Café {tag}"), format!("Cafe {tag}")),
        (format!("🚀 {tag}"), format!("🔥 {tag}")),
    ];

    for (first, second) in pairs {
        let one = store
            .create_folder(me, folder_name(&first))
            .await
            .unwrap_or_else(|error| panic!("{first}: {error}"));
        let other = store.create_folder(me, folder_name(&second)).await;

        assert!(other.is_ok(), "{second} beside {first}: {other:?}");
        for made in [Some(one), other.ok()].into_iter().flatten() {
            store
                .delete_folder(made.id)
                .await
                .unwrap_or_else(|error| panic!("{error}"));
        }
    }
}
