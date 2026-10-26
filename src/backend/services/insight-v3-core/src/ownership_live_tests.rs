use sea_orm::{DbBackend, FromQueryResult, Statement};
use serde_json::json;
use uuid::Uuid;

use crate::domain::access::{Access, Caller, Entry, Grantee, Level, Owner, PersonId, access};
use crate::domain::dashboards::{DashboardCatalogue as _, DashboardQuery};
use crate::domain::definition::{
    Change, DefinitionId, DefinitionKind, DefinitionLabel, Definitions, Page,
};
use crate::domain::folders::{FolderFilter, FolderId, FolderName, Folders};
use crate::domain::pins::Pins;
use crate::domain::sharing::{Grants, Handover, Shared};
use crate::domain::tags::{TagName, TagSet, Tags};
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

/// A tag name no other run holds, within the tag name length.
fn unique_tag(prefix: &str) -> String {
    let id = Uuid::now_v7().simple().to_string();

    format!("{prefix} {}", &id[id.len() - 12..])
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

async fn granted(store: &MariaDefinitions, shared: Shared<'_>, grantee: Grantee, level: Level) {
    store
        .grant(shared, grantee, level, a_person())
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

async fn folder_of(store: &MariaDefinitions, owner: PersonId) -> FolderId {
    store
        .create_folder(
            owner,
            FolderName::parse("Live").unwrap_or_else(|error| panic!("{error}")),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .id
}

async fn carries(db: &sea_orm::DatabaseConnection, folder: FolderId, board: &DefinitionId) -> i64 {
    TotalRow::find_by_statement(Statement::from_sql_and_values(
        DbBackend::MySql,
        "SELECT COUNT(*) AS total FROM folder_entries WHERE folder_id = ? AND dashboard = ? AND carries",
        [folder.to_string().into(), board.as_str().into()],
    ))
    .one(db)
    .await
    .unwrap_or_else(|error| panic!("{error}"))
    .map_or(-1, |row| row.total)
}

async fn remove(store: &MariaDefinitions, ids: &[&DefinitionId]) {
    for id in ids {
        Definitions::delete(store, DefinitionKind::Dashboard, id)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

async fn remove_folders(store: &MariaDefinitions, ids: &[FolderId]) {
    for id in ids {
        store
            .delete_folder(*id)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

#[tokio::test]
async fn the_ownership_schema_is_in_place_after_the_migrations_ran_twice() {
    let Some(db) = database_or_skip().await else {
        return;
    };
    let present = [
        (
            "SELECT COUNT(*) AS total FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'dashboards' AND column_name = 'owner_person_id'",
            1,
        ),
        (
            "SELECT COUNT(*) AS total FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'folders' AND column_name = 'owner_person_id'",
            1,
        ),
        (
            "SELECT COUNT(*) AS total FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'tags' AND column_name = 'owner_scope' AND extra LIKE '%STORED%'",
            1,
        ),
        (
            "SELECT COUNT(*) AS total FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = 'tags' AND index_name = 'tags_owner_name' AND non_unique = 0",
            2,
        ),
        (
            "SELECT COUNT(*) AS total FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name IN ('dashboard_grants', 'folder_grants', 'folder_entries')",
            3,
        ),
    ];
    let gone = [
        "SELECT COUNT(*) AS total FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = 'folders' AND index_name = 'folders_name'",
        "SELECT COUNT(*) AS total FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = 'tags' AND index_name = 'tags_name'",
        "SELECT COUNT(*) AS total FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name IN ('dashboard_shares', 'dashboard_placements')",
    ];

    for (sql, expected) in present {
        let found = TotalRow::find_by_statement(Statement::from_string(DbBackend::MySql, sql))
            .one(&db)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"))
            .map_or(0, |row| row.total);
        assert!(found >= expected, "{sql}: {found}");
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

/// What one caller is answered: the listing by `prefix`, each of `boards` by
/// one lookup, and the listing's total.
type Answered = (Caller, Vec<(String, Access)>, Vec<(String, Access)>, u64);

async fn answer_of(
    store: &MariaDefinitions,
    caller: Caller,
    prefix: &str,
    boards: &[DefinitionId],
) -> Answered {
    let query = DashboardQuery {
        needle: prefix.to_owned(),
        ..DashboardQuery::default()
    };
    let page = Page::parse(None, None).unwrap_or_else(|error| panic!("{error}"));
    let listed = store
        .page_dashboards(caller, &query, page)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let mut looked_up = Vec::new();
    for board in boards {
        let standing = store
            .standing(board, caller)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        if let Some(seen) = standing.and_then(|held| access(caller, &held)) {
            looked_up.push((board.as_str().to_owned(), seen));
        }
    }
    let paged = listed
        .items
        .into_iter()
        .map(|item| (item.id, item.access))
        .collect();

    (caller, paged, looked_up, listed.total)
}

#[tokio::test]
async fn the_listing_predicate_and_one_lookup_agree_for_every_caller() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (me, them, member) = (a_person(), a_person(), a_person());
    let prefix = unique("seen");
    let named = |rest: &str| dashboard(&format!("{prefix}_{rest}"));
    let boards = [
        named("a_mine"),
        named("b_granted"),
        named("c_everyone"),
        named("d_carried"),
        named("e_private"),
        named("f_legacy"),
    ];
    let [mine, granted_one, everyone, carried, private, legacy] = &boards;
    owned(&store, mine, Owner::Person(me)).await;
    for board in [granted_one, everyone, carried, private] {
        owned(&store, board, Owner::Person(them)).await;
    }
    owned(&store, legacy, Owner::Unowned).await;
    granted(
        &store,
        Shared::Dashboard(granted_one),
        Grantee::Person(me),
        Level::Editor,
    )
    .await;
    granted(
        &store,
        Shared::Dashboard(granted_one),
        Grantee::Person(member),
        Level::Editor,
    )
    .await;
    granted(
        &store,
        Shared::Dashboard(everyone),
        Grantee::Everyone,
        Level::Viewer,
    )
    .await;
    let team = folder_of(&store, them).await;
    granted(
        &store,
        Shared::Folder(team),
        Grantee::Person(member),
        Level::Viewer,
    )
    .await;
    store
        .add_entry(team, carried, Caller::administrator(them), Entry::Carrying)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let callers = [
        Caller::administrator(me),
        Caller::member(me),
        Caller::member(member),
    ];
    let mut answers = Vec::new();
    for caller in callers {
        answers.push(answer_of(&store, caller, &prefix, &boards).await);
    }
    remove(&store, &boards.iter().collect::<Vec<_>>()).await;
    remove_folders(&store, &[team]).await;
    for (caller, paged, looked_up, total) in &answers {
        assert_eq!(
            paged, looked_up,
            "{caller:?}: one lookup and the listing predicate agree"
        );
        assert_eq!(*total, paged.len() as u64, "{caller:?}");
    }
    let kinds: Vec<Vec<Access>> = answers
        .iter()
        .map(|(_, paged, _, _)| paged.iter().map(|(_, seen)| *seen).collect())
        .collect();
    assert_eq!(
        kinds,
        [
            vec![
                Access::Owner,
                Access::Editor,
                Access::Viewer,
                Access::Ownerless
            ],
            vec![Access::Viewer, Access::Viewer],
            vec![Access::Viewer, Access::Viewer, Access::Viewer],
        ]
    );
}

#[tokio::test]
async fn a_withdrawal_keeps_pins_and_entries_and_demotes_and_a_transfer_moves_only_once() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let Some(db) = database_or_skip().await else {
        return;
    };
    let (owner, editor, other) = (a_person(), a_person(), a_person());
    let board = dashboard(&unique("withdrawn"));
    owned(&store, &board, Owner::Person(owner)).await;
    let shared = Shared::Dashboard(&board);
    granted(&store, shared, Grantee::Person(editor), Level::Viewer).await;
    granted(&store, shared, Grantee::Person(editor), Level::Editor).await;
    let as_editor = Caller::administrator(editor);
    store
        .pin(as_editor, &board)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let theirs = folder_of(&store, editor).await;
    let mine = folder_of(&store, owner).await;
    store
        .add_entry(theirs, &board, as_editor, Entry::Carrying)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    store
        .add_entry(mine, &board, Caller::administrator(owner), Entry::Carrying)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let grants_before = store
        .grants_of(shared)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    store
        .withdraw(shared, Grantee::Person(editor))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let pins = store
        .pins_of(as_editor)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let unseen = store
        .page_dashboards(
            as_editor,
            &DashboardQuery {
                folder: Some(FolderFilter::In(theirs)),
                ..DashboardQuery::default()
            },
            Page::parse(None, None).unwrap_or_else(|error| panic!("{error}")),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let editor_entry = carries(&db, theirs, &board).await;
    let owner_entry = carries(&db, mine, &board).await;
    let still_filed = store
        .remove_entry(theirs, &board)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let moved = store
        .transfer(&board, owner, other)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let moved_again = store
        .transfer(&board, owner, editor)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let owner_entry_after = carries(&db, mine, &board).await;
    let now = store
        .standing(&board, Caller::administrator(other))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    remove(&store, &[&board]).await;
    remove_folders(&store, &[theirs, mine]).await;
    assert_eq!(grants_before.len(), 1, "granting again sets the level");
    assert_eq!(grants_before[0].level, Level::Editor);
    assert!(pins.is_empty(), "{pins:?}");
    assert_eq!(unseen.total, 0);
    assert_eq!(
        editor_entry, 0,
        "the withdrawn editor's entry stops carrying"
    );
    assert_eq!(owner_entry, 1, "the owner's entry carries on");
    assert!(
        still_filed,
        "the withdrawn editor's entry stays as a filing"
    );
    assert_eq!(moved, Handover::Handed);
    assert_eq!(
        moved_again,
        Handover::OwnerChanged,
        "the second transfer read an owner who no longer is"
    );
    assert_eq!(
        owner_entry_after, 0,
        "the former owner's entry stops carrying"
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
    let name = unique_tag("Scoped");
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

#[tokio::test]
async fn a_transfer_moves_the_tags_into_the_new_owners_scope_under_their_spelling() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let Some(db) = database_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let (former, heir) = (a_person(), a_person());
    let (handed, kept) = (dashboard(&unique("handed")), dashboard(&unique("kept")));
    owned(&store, &handed, Owner::Person(former)).await;
    owned(&store, &kept, Owner::Person(heir)).await;
    let name = unique_tag("Moved");
    for (board, owner, spelling) in [
        (&handed, former, name.to_lowercase()),
        (&kept, heir, name.clone()),
    ] {
        let wanted = TagSet::parse(&[spelling]).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_tags(board, Owner::Person(owner), &wanted)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }

    let moved = store
        .transfer(&handed, former, heir)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let carried = store
        .tags_of(&handed)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let left_in_former_scope = TotalRow::find_by_statement(Statement::from_sql_and_values(
        DbBackend::MySql,
        "SELECT COUNT(*) AS total FROM tags WHERE owner_scope = ?",
        [former.to_string().into()],
    ))
    .one(&db)
    .await
    .unwrap_or_else(|error| panic!("{error}"))
    .map_or(-1, |row| row.total);

    remove(&store, &[&handed, &kept]).await;
    assert_eq!(moved, Handover::Handed);
    assert_eq!(
        carried.iter().map(TagName::as_str).collect::<Vec<_>>(),
        [name.as_str()],
        "the new owner's spelling should win once the tag is in their scope"
    );
    assert_eq!(left_in_former_scope, 0, "the former owner's tag is pruned");
}
