use sea_orm::{DbBackend, FromQueryResult, Statement};
use serde_json::json;
use uuid::Uuid;

use crate::domain::access::{Access, Owner, PersonId};
use crate::domain::dashboards::{DashboardCatalogue as _, DashboardQuery, Owned};
use crate::domain::definition::{
    Change, DefinitionId, DefinitionKind, DefinitionLabel, Definitions, Page,
};
use crate::domain::folders::{FolderName, Folders};
use crate::domain::pins::Pins;
use crate::domain::sharing::Sharing;
use crate::domain::tags::{TagSet, Tags};
use crate::live_mariadb::{database_or_skip, store_or_skip, unique};
use crate::store::definitions::MariaDefinitions;
use crate::tags_live_tests::ONE_AT_A_TIME;

#[derive(Debug, FromQueryResult)]
struct TotalRow {
    total: i64,
}

fn a_person() -> PersonId {
    PersonId::new(Uuid::now_v7())
}

fn dashboard(name: &str) -> DefinitionId {
    DefinitionId::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

async fn owned(store: &MariaDefinitions, id: &DefinitionId, owner: Owner) {
    let mut changes = vec![Change::Create(
        DefinitionKind::Dashboard,
        id.clone(),
        DefinitionLabel::of_id(id),
        json!({ "title": "Board", "items": [] }),
    )];
    if let Owner::Person(person) = owner {
        changes.push(Change::Own {
            dashboard: id.clone(),
            owner: person,
        });
    }

    store
        .apply(&changes)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

async fn remove(store: &MariaDefinitions, ids: &[&DefinitionId]) {
    for id in ids {
        Definitions::delete(store, DefinitionKind::Dashboard, id)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

#[tokio::test]
async fn the_ownership_schema_is_in_place_after_the_migrations_ran_twice() {
    let Some(db) = database_or_skip().await else {
        return;
    };
    let parts = [
        "SELECT COUNT(*) AS total FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'dashboards' AND column_name = 'owner_person_id'",
        "SELECT COUNT(*) AS total FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'folders' AND column_name = 'owner_person_id'",
        "SELECT COUNT(*) AS total FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'tags' AND column_name = 'owner_scope' AND extra LIKE '%STORED%'",
        "SELECT COUNT(*) AS total FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = 'tags' AND index_name = 'tags_owner_name' AND non_unique = 0",
        "SELECT COUNT(*) AS total FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name IN ('dashboard_shares', 'dashboard_placements')",
    ];
    let gone = [
        "SELECT COUNT(*) AS total FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = 'folders' AND index_name = 'folders_name'",
        "SELECT COUNT(*) AS total FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = 'tags' AND index_name = 'tags_name'",
    ];

    for sql in parts {
        let found = TotalRow::find_by_statement(Statement::from_string(DbBackend::MySql, sql))
            .one(&db)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"))
            .map_or(0, |row| row.total);
        assert!(found > 0, "{sql}");
    }
    for sql in gone {
        let found = TotalRow::find_by_statement(Statement::from_string(DbBackend::MySql, sql))
            .one(&db)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"))
            .map_or(0, |row| row.total);
        assert_eq!(found, 0, "{sql}");
    }
}

#[tokio::test]
async fn a_caller_lists_their_own_the_shared_and_the_ownerless_and_never_a_private_one() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (me, them) = (a_person(), a_person());
    let prefix = unique("seen");
    let named = |rest: &str| dashboard(&format!("{prefix}_{rest}"));
    let (mine, shared, private, legacy) = (named("a"), named("b"), named("c"), named("d"));
    owned(&store, &mine, Owner::Person(me)).await;
    owned(&store, &shared, Owner::Person(them)).await;
    owned(&store, &private, Owner::Person(them)).await;
    owned(&store, &legacy, Owner::Unowned).await;
    store
        .share(&shared, me, them)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let listed = |owned_by: Option<Owned>, limit: u64, offset: u64| {
        let query = DashboardQuery {
            needle: prefix.clone(),
            owned: owned_by,
            ..DashboardQuery::default()
        };
        let page = Page::parse(Some(limit), Some(offset)).unwrap_or_else(|error| panic!("{error}"));
        let store = &store;
        async move {
            store
                .page_dashboards(me, &query, page)
                .await
                .unwrap_or_else(|error| panic!("{error}"))
        }
    };
    let everything = listed(None, 2, 0).await;
    let rest = listed(None, 2, 2).await;
    let only_mine = listed(Some(Owned::Me), 50, 0).await;
    let only_shared = listed(Some(Owned::Shared), 50, 0).await;
    let standings = [
        store.standing(&mine, me).await,
        store.standing(&shared, me).await,
        store.standing(&private, me).await,
        store.standing(&legacy, me).await,
    ];

    remove(&store, &[&mine, &shared, &private, &legacy]).await;
    let ids = |page: &crate::domain::dashboards::DashboardPage| -> Vec<String> {
        page.items.iter().map(|item| item.id.clone()).collect()
    };
    assert_eq!(everything.total, 3, "{everything:?}");
    assert_eq!(ids(&everything), [mine.as_str(), shared.as_str()]);
    assert_eq!(ids(&rest), [legacy.as_str()]);
    let accesses: Vec<Access> = everything
        .items
        .iter()
        .chain(&rest.items)
        .map(|item| item.access)
        .collect();
    assert_eq!(accesses, [Access::Owner, Access::Viewer, Access::Ownerless]);
    assert_eq!(ids(&only_mine), [mine.as_str()]);
    assert_eq!(ids(&only_shared), [shared.as_str()]);
    let seen: Vec<Option<Access>> = standings
        .into_iter()
        .map(|standing| {
            standing
                .unwrap_or_else(|error| panic!("{error}"))
                .and_then(|held| held.seen_by(me))
                .map(|seen| seen.access)
        })
        .collect();
    assert_eq!(
        seen,
        [
            Some(Access::Owner),
            Some(Access::Viewer),
            None,
            Some(Access::Ownerless)
        ],
        "one lookup and the listing predicate agree"
    );
}

#[tokio::test]
async fn a_withdrawal_takes_the_share_pin_and_placement_and_a_transfer_moves_only_once() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (owner, viewer, other) = (a_person(), a_person(), a_person());
    let board = dashboard(&unique("withdrawn"));
    owned(&store, &board, Owner::Person(owner)).await;
    store
        .share(&board, viewer, owner)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    store
        .share(&board, viewer, owner)
        .await
        .unwrap_or_else(|error| panic!("a repeated share is the same share: {error}"));
    store
        .pin(viewer, &board)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let folder = store
        .create_folder(
            viewer,
            FolderName::parse("Viewer's").unwrap_or_else(|error| panic!("{error}")),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    store
        .file(&board, Owner::Person(viewer), Some(folder.id))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let shares_before = store
        .shares_of(&board)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    store
        .withdraw(&board, viewer)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let pins = store
        .pins_of(viewer)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let placed = store
        .folder_of(&board, viewer)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let moved = store
        .transfer(&board, owner, other)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let moved_again = store
        .transfer(&board, owner, viewer)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let now = store
        .standing(&board, other)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    remove(&store, &[&board]).await;
    store
        .delete_folder(viewer, folder.id)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(shares_before.len(), 1);
    assert!(pins.is_empty(), "{pins:?}");
    assert_eq!(placed, None);
    assert!(moved);
    assert!(
        !moved_again,
        "the second transfer read an owner who no longer is"
    );
    assert_eq!(now.map(|held| held.owner), Some(Owner::Person(other)));
}

#[tokio::test]
async fn a_tag_name_is_unique_within_an_owner_and_shared_between_owners() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let (me, them) = (a_person(), a_person());
    let (mine, theirs) = (dashboard(&unique("mine")), dashboard(&unique("theirs")));
    owned(&store, &mine, Owner::Person(me)).await;
    owned(&store, &theirs, Owner::Person(them)).await;
    let name = format!("Scoped {}", Uuid::now_v7().simple());
    let wanted =
        TagSet::parse(std::slice::from_ref(&name)).unwrap_or_else(|error| panic!("{error}"));

    let first = store.set_tags(&mine, Owner::Person(me), &wanted).await;
    let second = store.set_tags(&theirs, Owner::Person(them), &wanted).await;
    let again = store.set_tags(&mine, Owner::Person(me), &wanted).await;

    remove(&store, &[&mine, &theirs]).await;
    for answer in [&first, &second, &again] {
        assert!(answer.is_ok(), "{answer:?}");
    }
}
