use std::error::Error;

use serde_json::json;

use super::*;
use crate::domain::access::{Access, FolderAccess};
use crate::domain::dashboards::{DashboardCatalogue as _, DashboardPage, DashboardQuery, Owned};
use crate::domain::folders::{FolderError, FolderFilter, FolderName, Folders};
use crate::domain::sharing::{Grants as _, Shared};

type R = Result<(), Box<dyn Error>>;

fn name(value: &str) -> DefinitionId {
    DefinitionId::parse(value).unwrap_or_else(|error| panic!("the name must parse: {error}"))
}

fn folder_name(value: &str) -> FolderName {
    FolderName::parse(value).unwrap_or_else(|error| panic!("the folder name must parse: {error}"))
}

fn me() -> PersonId {
    PersonId::new(uuid::Uuid::from_u128(1))
}

fn someone_else() -> PersonId {
    PersonId::new(uuid::Uuid::from_u128(2))
}

fn a_member() -> PersonId {
    PersonId::new(uuid::Uuid::from_u128(3))
}

fn admin(person: PersonId) -> Caller {
    Caller::administrator(person)
}

fn member(person: PersonId) -> Caller {
    Caller::member(person)
}

/// Boards of `owner`'s, under ids the case names.
async fn boards_of(store: &MemoryDefinitions, owner: PersonId, names: &[&str]) {
    for board in names {
        store
            .apply(&[
                Change::Create(
                    DefinitionKind::Dashboard,
                    name(board),
                    label(board),
                    json!({ "title": board }),
                ),
                Change::Own {
                    dashboard: name(board),
                    owner,
                },
            ])
            .await
            .unwrap_or_else(|error| panic!("`{board}` must store: {error}"));
    }
}

/// Boards of mine, under ids the case names.
async fn with_boards(names: &[&str]) -> MemoryDefinitions {
    let store = MemoryDefinitions::new();
    boards_of(&store, me(), names).await;
    store
}

async fn grant(store: &MemoryDefinitions, shared: Shared<'_>, grantee: Grantee, level: Level) -> R {
    store.grant(shared, grantee, level, me()).await?;
    Ok(())
}

fn counts(list: &crate::domain::folders::FolderList) -> Vec<(String, u64)> {
    list.folders
        .iter()
        .map(|summary| {
            (
                summary.seen.folder.name.as_str().to_owned(),
                summary.dashboards,
            )
        })
        .collect()
}

async fn access_of(
    store: &MemoryDefinitions,
    dashboard: &str,
    caller: Caller,
) -> Result<Option<Access>, Box<dyn Error>> {
    Ok(store
        .standing(&name(dashboard), caller)
        .await?
        .and_then(|standing| access(caller, &standing)))
}

async fn seen_ids(
    store: &MemoryDefinitions,
    caller: Caller,
    query: &DashboardQuery,
) -> Vec<String> {
    let page = Page::parse(None, None).unwrap_or_else(|error| panic!("{error}"));

    store
        .page_dashboards(caller, query, page)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .items
        .into_iter()
        .map(|listed| listed.id)
        .collect()
}

#[tokio::test]
async fn a_new_folder_is_listed_empty_and_owned() -> R {
    let store = with_boards(&["delivery"]).await;

    store.create_folder(me(), folder_name("Platform")).await?;

    let listed = store.list_folders(admin(me())).await?;
    assert_eq!(counts(&listed), vec![("Platform".to_owned(), 0)]);
    assert_eq!(listed.folders[0].seen.access, FolderAccess::Owner);
    assert_eq!(listed.unfiled, 1);
    Ok(())
}

#[tokio::test]
async fn a_folder_is_seen_by_its_owner_and_its_grantees_only() -> R {
    let store = MemoryDefinitions::new();
    let private = store
        .create_folder(someone_else(), folder_name("Private"))
        .await?;
    let shared = store
        .create_folder(someone_else(), folder_name("Shared"))
        .await?;
    let open = store
        .create_folder(someone_else(), folder_name("Open"))
        .await?;
    let legacy = store.seed_ownerless_folder("Legacy");
    grant(
        &store,
        Shared::Folder(shared.id),
        Grantee::Person(a_member()),
        Level::Editor,
    )
    .await?;
    grant(
        &store,
        Shared::Folder(open.id),
        Grantee::Everyone,
        Level::Viewer,
    )
    .await?;

    let as_member = store.list_folders(member(a_member())).await?;
    let as_admin = store.list_folders(admin(me())).await?;

    let seen: Vec<(String, FolderAccess)> = as_member
        .folders
        .iter()
        .map(|summary| {
            (
                summary.seen.folder.name.as_str().to_owned(),
                summary.seen.access,
            )
        })
        .collect();
    assert_eq!(
        seen,
        vec![
            ("Open".to_owned(), FolderAccess::Viewer),
            ("Shared".to_owned(), FolderAccess::Viewer),
        ],
        "a member is a viewer even where granted editor, and never sees an ownerless folder"
    );
    let names: Vec<String> = as_admin
        .folders
        .iter()
        .map(|summary| summary.seen.folder.name.as_str().to_owned())
        .collect();
    assert_eq!(names, vec!["Legacy", "Open"]);
    assert!(
        store.held_folder(private.id, admin(me())).await?.is_some(),
        "the row is there; only the access decides"
    );
    let _ = legacy;
    Ok(())
}

#[tokio::test]
async fn a_carrying_entry_shows_the_dashboard_to_the_folder_s_audience() -> R {
    let store = with_boards(&["delivery", "support"]).await;
    let team = store.create_folder(me(), folder_name("Team")).await?;
    grant(
        &store,
        Shared::Folder(team.id),
        Grantee::Person(someone_else()),
        Level::Editor,
    )
    .await?;
    grant(
        &store,
        Shared::Folder(team.id),
        Grantee::Person(a_member()),
        Level::Viewer,
    )
    .await?;

    let entry =
        crate::domain::folders::add(&store, admin(me()), team.id, &name("delivery")).await?;

    assert_eq!(entry, Entry::Carrying);
    assert_eq!(
        access_of(&store, "delivery", admin(someone_else())).await?,
        Some(Access::Editor)
    );
    assert_eq!(
        access_of(&store, "delivery", member(a_member())).await?,
        Some(Access::Viewer)
    );
    assert_eq!(
        access_of(&store, "support", admin(someone_else())).await?,
        None
    );
    assert_eq!(
        counts(&store.list_folders(member(a_member())).await?),
        vec![("Team".to_owned(), 1)]
    );
    Ok(())
}

#[tokio::test]
async fn a_filing_entry_carries_nothing_and_shows_only_to_who_sees_the_dashboard() -> R {
    let store = MemoryDefinitions::new();
    boards_of(&store, someone_else(), &["delivery"]).await;
    grant(
        &store,
        Shared::Dashboard(&name("delivery")),
        Grantee::Person(a_member()),
        Level::Viewer,
    )
    .await?;
    let personal = store.create_folder(a_member(), folder_name("Mine")).await?;
    grant(
        &store,
        Shared::Folder(personal.id),
        Grantee::Person(me()),
        Level::Viewer,
    )
    .await?;

    let entry =
        crate::domain::folders::add(&store, member(a_member()), personal.id, &name("delivery"))
            .await?;

    assert_eq!(entry, Entry::Filing);
    assert_eq!(store.carries(personal.id, &name("delivery")), Some(false));
    assert_eq!(access_of(&store, "delivery", admin(me())).await?, None);
    assert_eq!(
        counts(&store.list_folders(admin(me())).await?),
        vec![("Mine".to_owned(), 0)],
        "the folder's other viewers do not see what was only filed there"
    );
    assert_eq!(
        counts(&store.list_folders(member(a_member())).await?),
        vec![("Mine".to_owned(), 1)]
    );
    Ok(())
}

#[tokio::test]
async fn an_editor_through_a_folder_files_rather_than_carries() -> R {
    let store = with_boards(&["delivery"]).await;
    let team = store.create_folder(me(), folder_name("Team")).await?;
    grant(
        &store,
        Shared::Folder(team.id),
        Grantee::Person(someone_else()),
        Level::Editor,
    )
    .await?;
    crate::domain::folders::add(&store, admin(me()), team.id, &name("delivery")).await?;
    let theirs = store
        .create_folder(someone_else(), folder_name("Theirs"))
        .await?;

    let entry =
        crate::domain::folders::add(&store, admin(someone_else()), theirs.id, &name("delivery"))
            .await?;

    assert_eq!(
        access_of(&store, "delivery", admin(someone_else())).await?,
        Some(Access::Editor)
    );
    assert_eq!(entry, Entry::Filing);
    assert_eq!(store.carries(theirs.id, &name("delivery")), Some(false));
    Ok(())
}

#[tokio::test]
async fn withdrawing_an_editor_s_grant_stops_their_entry_carrying() -> R {
    let store = with_boards(&["delivery"]).await;
    let editor = someone_else();
    grant(
        &store,
        Shared::Dashboard(&name("delivery")),
        Grantee::Person(editor),
        Level::Editor,
    )
    .await?;
    let theirs = store.create_folder(editor, folder_name("Theirs")).await?;
    grant(
        &store,
        Shared::Folder(theirs.id),
        Grantee::Person(a_member()),
        Level::Viewer,
    )
    .await?;
    let entry =
        crate::domain::folders::add(&store, admin(editor), theirs.id, &name("delivery")).await?;
    assert_eq!(entry, Entry::Carrying);
    assert_eq!(
        access_of(&store, "delivery", member(a_member())).await?,
        Some(Access::Viewer)
    );

    store
        .withdraw(
            Shared::Dashboard(&name("delivery")),
            Grantee::Person(editor),
        )
        .await?;

    assert_eq!(store.carries(theirs.id, &name("delivery")), Some(false));
    assert_eq!(access_of(&store, "delivery", admin(editor)).await?, None);
    assert_eq!(
        access_of(&store, "delivery", member(a_member())).await?,
        None
    );
    Ok(())
}

#[tokio::test]
async fn lowering_an_everyone_editor_grant_stops_entries_relying_on_it() -> R {
    let store = with_boards(&["delivery"]).await;
    grant(
        &store,
        Shared::Dashboard(&name("delivery")),
        Grantee::Everyone,
        Level::Editor,
    )
    .await?;
    let theirs = store
        .create_folder(someone_else(), folder_name("Theirs"))
        .await?;
    crate::domain::folders::add(&store, admin(someone_else()), theirs.id, &name("delivery"))
        .await?;
    assert_eq!(store.carries(theirs.id, &name("delivery")), Some(true));

    grant(
        &store,
        Shared::Dashboard(&name("delivery")),
        Grantee::Everyone,
        Level::Viewer,
    )
    .await?;

    assert_eq!(store.carries(theirs.id, &name("delivery")), Some(false));
    Ok(())
}

#[tokio::test]
async fn a_transfer_stops_the_former_owner_s_entries_carrying() -> R {
    let store = MemoryDefinitions::new();
    boards_of(&store, someone_else(), &["delivery"]).await;
    let theirs = store
        .create_folder(someone_else(), folder_name("Theirs"))
        .await?;
    crate::domain::folders::add(&store, admin(someone_else()), theirs.id, &name("delivery"))
        .await?;
    assert_eq!(store.carries(theirs.id, &name("delivery")), Some(true));

    let handed = store
        .transfer(&name("delivery"), someone_else(), me())
        .await?;

    assert_eq!(handed, crate::domain::sharing::Handover::Handed);
    assert_eq!(store.carries(theirs.id, &name("delivery")), Some(false));
    Ok(())
}

#[tokio::test]
async fn a_dashboard_may_sit_in_several_folders_and_leaving_one_keeps_the_rest() -> R {
    let store = with_boards(&["delivery"]).await;
    let first = store.create_folder(me(), folder_name("First")).await?;
    let second = store.create_folder(me(), folder_name("Second")).await?;
    for folder in [first.id, second.id] {
        crate::domain::folders::add(&store, admin(me()), folder, &name("delivery")).await?;
    }

    crate::domain::folders::remove(&store, admin(me()), first.id, &name("delivery")).await?;

    let holding: Vec<String> = store
        .folders_holding(&name("delivery"), admin(me()))
        .await?
        .into_iter()
        .map(|folder| folder.name.as_str().to_owned())
        .collect();
    assert_eq!(holding, vec!["Second"]);
    assert!(
        store
            .read(DefinitionKind::Dashboard, &name("delivery"))
            .await?
            .is_some()
    );
    Ok(())
}

#[tokio::test]
async fn deleting_a_folder_leaves_its_dashboards_unfiled() -> R {
    let store = with_boards(&["delivery"]).await;
    let platform = store.create_folder(me(), folder_name("Platform")).await?;
    crate::domain::folders::add(&store, admin(me()), platform.id, &name("delivery")).await?;

    crate::domain::folders::delete(&store, admin(me()), platform.id).await?;

    let listed = store.list_folders(admin(me())).await?;
    assert!(listed.folders.is_empty());
    assert_eq!(listed.unfiled, 1);
    Ok(())
}

#[tokio::test]
async fn a_folder_editor_renames_and_arranges_but_does_not_delete() -> R {
    let store = with_boards(&["delivery"]).await;
    let team = store.create_folder(me(), folder_name("Team")).await?;
    grant(
        &store,
        Shared::Folder(team.id),
        Grantee::Person(someone_else()),
        Level::Editor,
    )
    .await?;
    let editor = admin(someone_else());

    crate::domain::folders::rename(&store, editor, team.id, folder_name("Renamed")).await?;
    let removed = crate::domain::folders::delete(&store, editor, team.id).await;
    let as_viewer =
        crate::domain::folders::rename(&store, member(someone_else()), team.id, folder_name("X"))
            .await;

    assert!(matches!(removed, Err(FolderError::NotOwner)), "{removed:?}");
    assert!(
        matches!(as_viewer, Err(FolderError::NotEditor)),
        "{as_viewer:?}"
    );
    assert_eq!(
        counts(&store.list_folders(admin(me())).await?),
        vec![("Renamed".to_owned(), 0)]
    );
    Ok(())
}

#[tokio::test]
async fn someone_elses_unshared_folder_is_not_found() -> R {
    let store = with_boards(&["delivery"]).await;
    let theirs = store
        .create_folder(someone_else(), folder_name("Theirs"))
        .await?;

    let renamed =
        crate::domain::folders::rename(&store, admin(me()), theirs.id, folder_name("Mine")).await;
    let added =
        crate::domain::folders::add(&store, admin(me()), theirs.id, &name("delivery")).await;

    for refused in [renamed.map(|_| ()), added.map(|_| ())] {
        assert!(
            matches!(refused, Err(FolderError::FolderNotFound(_))),
            "{refused:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_ownerless_dashboard_sits_only_in_an_ownerless_folder() -> R {
    let store = with_boards(&["owned"]).await;
    store
        .seed(
            DefinitionKind::Dashboard,
            &name("legacy"),
            &json!({ "title": "Legacy" }),
        )
        .await?;
    let legacy = store.seed_ownerless_folder("Legacy");
    let mine = store.create_folder(me(), folder_name("Mine")).await?;

    let fits = crate::domain::folders::add(&store, admin(me()), legacy, &name("legacy")).await;
    let ownerless_in_mine =
        crate::domain::folders::add(&store, admin(me()), mine.id, &name("legacy")).await;
    let owned_in_legacy =
        crate::domain::folders::add(&store, admin(me()), legacy, &name("owned")).await;

    assert_eq!(fits?, Entry::Carrying);
    for refused in [ownerless_in_mine, owned_in_legacy] {
        assert!(
            matches!(refused, Err(FolderError::WrongShelf(_))),
            "{refused:?}"
        );
    }
    assert_eq!(access_of(&store, "legacy", member(a_member())).await?, None);
    Ok(())
}

#[tokio::test]
async fn a_page_keeps_only_the_dashboards_the_caller_sees() -> R {
    let store = with_boards(&["mine"]).await;
    boards_of(&store, someone_else(), &["shared", "everyone", "private"]).await;
    grant(
        &store,
        Shared::Dashboard(&name("shared")),
        Grantee::Person(me()),
        Level::Viewer,
    )
    .await?;
    grant(
        &store,
        Shared::Dashboard(&name("everyone")),
        Grantee::Everyone,
        Level::Editor,
    )
    .await?;
    store
        .seed(
            DefinitionKind::Dashboard,
            &name("legacy"),
            &json!({ "title": "Legacy" }),
        )
        .await?;

    let seen = store
        .page_dashboards(
            admin(me()),
            &DashboardQuery::default(),
            Page::parse(None, None)?,
        )
        .await?;
    let by_member = seen_ids(&store, member(me()), &DashboardQuery::default()).await;

    assert_eq!(
        dashboard_ids(&seen),
        vec!["everyone", "legacy", "mine", "shared"]
    );
    assert_eq!(seen.total, 4);
    let accesses: Vec<&str> = seen.items.iter().map(|item| item.access.as_str()).collect();
    assert_eq!(accesses, vec!["editor", "ownerless", "owner", "viewer"]);
    assert_eq!(
        by_member,
        vec!["everyone", "shared"],
        "a member sees no ownerless row and owns nothing"
    );
    Ok(())
}

#[tokio::test]
async fn owned_by_me_and_shared_are_told_apart() -> R {
    let store = with_boards(&["mine"]).await;
    boards_of(&store, someone_else(), &["shared"]).await;
    grant(
        &store,
        Shared::Dashboard(&name("shared")),
        Grantee::Person(me()),
        Level::Viewer,
    )
    .await?;
    let owned = |owned| DashboardQuery {
        owned: Some(owned),
        ..DashboardQuery::default()
    };

    assert_eq!(
        seen_ids(&store, admin(me()), &owned(Owned::Me)).await,
        vec!["mine"]
    );
    assert_eq!(
        seen_ids(&store, admin(me()), &owned(Owned::Shared)).await,
        vec!["shared"]
    );
    Ok(())
}

#[tokio::test]
async fn a_folder_filter_and_unfiled_read_only_the_folders_the_caller_sees() -> R {
    let store = with_boards(&["delivery", "delivery_ai", "support"]).await;
    grant(
        &store,
        Shared::Dashboard(&name("support")),
        Grantee::Person(someone_else()),
        Level::Viewer,
    )
    .await?;
    let platform = store.create_folder(me(), folder_name("Platform")).await?;
    for board in ["delivery", "support"] {
        crate::domain::folders::add(&store, admin(me()), platform.id, &name(board)).await?;
    }

    let filed = seen_ids(
        &store,
        admin(me()),
        &in_folder("", FolderFilter::In(platform.id)),
    )
    .await;
    let searched = seen_ids(
        &store,
        admin(me()),
        &in_folder("deliv", FolderFilter::In(platform.id)),
    )
    .await;
    let unfiled = seen_ids(&store, admin(me()), &in_folder("", FolderFilter::Unfiled)).await;
    let theirs_unfiled = seen_ids(
        &store,
        admin(someone_else()),
        &in_folder("", FolderFilter::Unfiled),
    )
    .await;
    let theirs_filed = seen_ids(
        &store,
        admin(someone_else()),
        &in_folder("", FolderFilter::In(platform.id)),
    )
    .await;

    assert_eq!(filed, vec!["delivery", "support"]);
    assert_eq!(searched, vec!["delivery"]);
    assert_eq!(unfiled, vec!["delivery_ai"]);
    assert_eq!(theirs_unfiled, vec!["support"], "they do not see my folder");
    assert!(theirs_filed.is_empty(), "{theirs_filed:?}");
    Ok(())
}

#[tokio::test]
async fn withdrawing_a_grant_hides_pins_and_entries_and_granting_again_shows_them() -> R {
    use crate::domain::pins::Pins as _;

    let store = MemoryDefinitions::new();
    boards_of(&store, someone_else(), &["delivery"]).await;
    let shared = Shared::Dashboard(&name("delivery"));
    grant(&store, shared, Grantee::Person(a_member()), Level::Viewer).await?;
    let caller = member(a_member());
    let personal = store.create_folder(a_member(), folder_name("Mine")).await?;
    crate::domain::folders::add(&store, caller, personal.id, &name("delivery")).await?;
    store.pin(caller, &name("delivery")).await?;

    store.withdraw(shared, Grantee::Person(a_member())).await?;
    let hidden_pins = store.pins_of(caller).await?;
    let hidden_count = counts(&store.list_folders(caller).await?);
    grant(&store, shared, Grantee::Person(a_member()), Level::Viewer).await?;

    assert!(hidden_pins.is_empty(), "{hidden_pins:?}");
    assert_eq!(hidden_count, vec![("Mine".to_owned(), 0)]);
    assert_eq!(store.pins_of(caller).await?.len(), 1);
    assert_eq!(
        counts(&store.list_folders(caller).await?),
        vec![("Mine".to_owned(), 1)]
    );
    Ok(())
}

#[tokio::test]
async fn an_orphaned_folder_is_one_shared_whose_owner_is_no_administrator() -> R {
    let store = MemoryDefinitions::new();
    let shared = store
        .create_folder(someone_else(), folder_name("Shared"))
        .await?;
    store
        .create_folder(someone_else(), folder_name("Private"))
        .await?;
    store.create_folder(me(), folder_name("Kept")).await?;
    grant(
        &store,
        Shared::Folder(shared.id),
        Grantee::Everyone,
        Level::Viewer,
    )
    .await?;

    let orphans = store
        .page_orphaned_folders(&[me()], Page::parse(None, None)?)
        .await?;

    let names: Vec<&str> = orphans
        .items
        .iter()
        .map(|orphan| orphan.name.as_str())
        .collect();
    assert_eq!(names, vec!["Shared"]);
    assert_eq!(orphans.total, 1);
    Ok(())
}

#[tokio::test]
async fn a_carried_entry_moves_to_the_copy_only_where_the_placer_may_arrange() -> R {
    let store = with_boards(&["original"]).await;
    let platform = store.create_folder(me(), folder_name("Platform")).await?;
    let viewed = store
        .create_folder(someone_else(), folder_name("Viewed"))
        .await?;
    grant(
        &store,
        Shared::Folder(viewed.id),
        Grantee::Person(me()),
        Level::Viewer,
    )
    .await?;
    for folder in [platform.id, viewed.id] {
        store
            .add_entry(folder, &name("original"), admin(me()), Entry::Carrying)
            .await?;
    }

    store
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                name("copy"),
                label("Copy"),
                json!({ "title": "original" }),
            ),
            Change::Own {
                dashboard: name("copy"),
                owner: me(),
            },
            Change::CarryEntries {
                from: name("original"),
                to: name("copy"),
                placer: admin(me()),
            },
        ])
        .await?;

    assert_eq!(store.carries(platform.id, &name("copy")), Some(true));
    assert_eq!(store.carries(viewed.id, &name("copy")), None);
    Ok(())
}

fn dashboard_ids(page: &DashboardPage) -> Vec<&str> {
    page.items.iter().map(|listed| listed.id.as_str()).collect()
}

fn in_folder(needle: &str, filter: FolderFilter) -> DashboardQuery {
    DashboardQuery {
        needle: needle.to_owned(),
        folder: Some(filter),
        ..DashboardQuery::default()
    }
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
        .page(LibraryKind::Widget, "", Page::parse(None, None)?)
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
        .page(LibraryKind::Metric, "lead", Page::parse(None, None)?)
        .await?;

    assert_eq!(ids(&found), vec!["m1"]);

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
async fn a_folder_past_one_persons_cap_is_refused_and_another_person_is_not() -> R {
    let store = MemoryDefinitions::new();
    for index in 0..crate::domain::folders::MAX_FOLDERS {
        store
            .create_folder(me(), folder_name(&format!("f{index}")))
            .await?;
    }

    let refused = store.create_folder(me(), folder_name("one more")).await;
    let theirs = store
        .create_folder(someone_else(), folder_name("one more"))
        .await;

    assert!(matches!(refused, Err(FolderError::TooMany)), "{refused:?}");
    assert!(theirs.is_ok(), "{theirs:?}");

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
