use sea_orm::{DbBackend, FromQueryResult, Statement};
use serde_json::json;
use uuid::Uuid;

use crate::domain::access::{Access, Caller, Entry, Grantee, Level, Owner, PersonId, access};
use crate::domain::dashboards::{DashboardCatalogue as _, DashboardQuery, Order, Visibility};
use crate::domain::definition::{
    Change, DefinitionId, DefinitionKind, DefinitionLabel, Definitions, Page,
};
use crate::domain::folders::{
    FolderError, FolderFilter, FolderId, FolderName, Folders, MAX_FOLDERS,
};
use crate::domain::pins::Pins;
use crate::domain::sharing::{Granting, Grants, Handover, Shared, Withdrawal};
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

async fn owner_of(store: &MariaDefinitions, shared: Shared<'_>) -> PersonId {
    let anyone = Caller::administrator(a_person());
    let owner = match shared {
        Shared::Dashboard(id) => store
            .standing(id, anyone)
            .await
            .unwrap_or_else(|error| panic!("{error}"))
            .map(|standing| standing.owner),
        Shared::Folder(id) => store
            .held_folder(id, anyone)
            .await
            .unwrap_or_else(|error| panic!("{error}"))
            .map(|held| held.folder.owner),
    };

    owner
        .and_then(Owner::person)
        .unwrap_or_else(|| panic!("an owned thing is granted"))
}

async fn granted(store: &MariaDefinitions, shared: Shared<'_>, grantee: Grantee, level: Level) {
    let by = owner_of(store, shared).await;
    let granting = store
        .grant(shared, grantee, level, by)
        .await
        .unwrap_or_else(|error| panic!("{error:?}"));

    assert_eq!(granting, Granting::Done, "the owner grants it");
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
async fn a_listing_reads_each_dashboards_visibility_from_its_own_grants() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (owner, other) = (a_person(), a_person());
    let prefix = unique("reach");
    let named = |rest: &str| dashboard(&format!("{prefix}_{rest}"));
    let boards = [named("a_private"), named("b_people"), named("c_everyone")];
    let [private, people, everyone] = &boards;
    for board in &boards {
        owned(&store, board, Owner::Person(owner)).await;
    }
    let grants = [
        (people, Grantee::Person(other)),
        (everyone, Grantee::Person(other)),
        (everyone, Grantee::Everyone),
    ];
    for (board, grantee) in grants {
        granted(&store, Shared::Dashboard(board), grantee, Level::Viewer).await;
    }

    let query = DashboardQuery {
        needle: prefix.clone(),
        ..DashboardQuery::default()
    };
    let page = Page::parse(None, None).unwrap_or_else(|error| panic!("{error}"));
    let listed = store
        .page_dashboards(Caller::administrator(owner), &query, page)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    remove(&store, &boards.iter().collect::<Vec<_>>()).await;

    let reach: Vec<(String, Visibility)> = listed
        .items
        .into_iter()
        .map(|item| (item.id, item.visibility))
        .collect();
    assert_eq!(
        reach,
        [
            (private.as_str().to_owned(), Visibility::Private),
            (people.as_str().to_owned(), Visibility::People),
            (everyone.as_str().to_owned(), Visibility::Everyone),
        ]
    );
}

async fn listed(
    store: &MariaDefinitions,
    caller: Caller,
    prefix: &str,
    order: Order,
) -> crate::domain::dashboards::DashboardPage {
    let query = DashboardQuery {
        needle: prefix.to_owned(),
        order,
        ..DashboardQuery::default()
    };
    let page = Page::parse(None, None).unwrap_or_else(|error| panic!("{error}"));

    store
        .page_dashboards(caller, &query, page)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
}

#[tokio::test]
async fn a_listing_counts_carrying_folders_in_visibility_and_sorts_by_last_change() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (owner, other) = (a_person(), a_person());
    let prefix = unique("carried");
    let named = |rest: &str| dashboard(&format!("{prefix}_{rest}"));
    let boards = [
        named("a_to_people"),
        named("b_to_everyone"),
        named("c_into_theirs"),
        named("d_filed"),
    ];
    let [to_people, to_everyone, into_theirs, filed] = &boards;
    for board in &boards {
        owned(&store, board, Owner::Person(owner)).await;
    }
    let team = folder_of(&store, owner).await;
    let org = folder_of(&store, owner).await;
    let theirs = folder_of(&store, other).await;
    granted(
        &store,
        Shared::Folder(team),
        Grantee::Person(other),
        Level::Viewer,
    )
    .await;
    granted(
        &store,
        Shared::Folder(org),
        Grantee::Everyone,
        Level::Viewer,
    )
    .await;
    granted(
        &store,
        Shared::Dashboard(filed),
        Grantee::Person(other),
        Level::Viewer,
    )
    .await;
    let as_owner = Caller::administrator(owner);
    let as_other = Caller::administrator(other);
    for (folder, board, caller, entry) in [
        (team, to_people, as_owner, Entry::Carrying),
        (org, to_everyone, as_owner, Entry::Carrying),
        (theirs, into_theirs, as_owner, Entry::Carrying),
        (theirs, filed, as_other, Entry::Filing),
    ] {
        store
            .add_entry(folder, board, caller, entry)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
    store
        .withdraw(Shared::Dashboard(filed), Grantee::Person(other), owner)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    store
        .apply(&[Change::Replace(
            DefinitionKind::Dashboard,
            to_people.clone(),
            DefinitionLabel::of_id(to_people),
            json!({ "title": "Changed", "items": [] }),
        )])
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let by_name = listed(&store, as_owner, &prefix, Order::Name).await;
    let recent = listed(&store, as_owner, &prefix, Order::Recent).await;
    remove(&store, &boards.iter().collect::<Vec<_>>()).await;
    remove_folders(&store, &[team, org, theirs]).await;

    let reach: Vec<(String, Visibility)> = by_name
        .items
        .into_iter()
        .map(|item| (item.id, item.visibility))
        .collect();
    assert_eq!(
        reach,
        [
            (to_people.as_str().to_owned(), Visibility::People),
            (to_everyone.as_str().to_owned(), Visibility::Everyone),
            (into_theirs.as_str().to_owned(), Visibility::People),
            (filed.as_str().to_owned(), Visibility::Private),
        ]
    );
    assert_eq!(
        recent.items.first().map(|item| item.id.as_str()),
        Some(to_people.as_str()),
        "the last changed comes first"
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
        .withdraw(shared, Grantee::Person(editor), owner)
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
        .remove_entry(theirs, &board, Caller::administrator(owner))
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

#[derive(Debug, FromQueryResult)]
struct PlacerRow {
    placed_by: Option<String>,
}

async fn placer(
    db: &sea_orm::DatabaseConnection,
    folder: FolderId,
    board: &DefinitionId,
) -> Option<String> {
    PlacerRow::find_by_statement(Statement::from_sql_and_values(
        DbBackend::MySql,
        "SELECT placed_by FROM folder_entries WHERE folder_id = ? AND dashboard = ?",
        [folder.to_string().into(), board.as_str().into()],
    ))
    .one(db)
    .await
    .unwrap_or_else(|error| panic!("{error}"))
    .and_then(|row| row.placed_by)
}

#[tokio::test]
async fn a_grant_change_lands_only_while_its_maker_stands_to_share_it() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (owner, editor, member) = (a_person(), a_person(), a_person());
    let board = dashboard(&unique("standing"));
    owned(&store, &board, Owner::Person(owner)).await;
    let shared = Shared::Dashboard(&board);
    granted(&store, shared, Grantee::Person(editor), Level::Viewer).await;

    let as_viewer = store
        .grant(shared, Grantee::Person(member), Level::Viewer, editor)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let withdrawn_as_viewer = store
        .withdraw(shared, Grantee::Person(editor), editor)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let to_owner = store
        .grant(shared, Grantee::Person(owner), Level::Viewer, owner)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    granted(&store, shared, Grantee::Person(editor), Level::Editor).await;
    let as_editor = store
        .grant(shared, Grantee::Person(member), Level::Viewer, editor)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let own_withdrawn = store
        .withdraw(shared, Grantee::Person(editor), editor)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let after_withdrawal = store
        .withdraw(shared, Grantee::Person(member), editor)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let grants = store
        .grants_of(shared)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let missing = store
        .grant(
            Shared::Dashboard(&dashboard(&unique("missing"))),
            Grantee::Everyone,
            Level::Viewer,
            owner,
        )
        .await;

    remove(&store, &[&board]).await;
    assert_eq!(as_viewer, Granting::NotSharer);
    assert_eq!(withdrawn_as_viewer, Withdrawal::NotSharer);
    assert_eq!(to_owner, Granting::ToOwner);
    assert_eq!(as_editor, Granting::Done);
    assert_eq!(own_withdrawn, Withdrawal::Done);
    assert_eq!(after_withdrawal, Withdrawal::NotSharer);
    assert_eq!(grants.len(), 1, "{grants:?}");
    assert_eq!(grants[0].grantee, Grantee::Person(member));
    assert!(missing.is_err(), "{missing:?}");
}

#[tokio::test]
async fn an_entry_carries_only_while_its_placer_qualifies_and_answers_what_it_stored() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let Some(db) = database_or_skip().await else {
        return;
    };
    let (owner, viewer) = (a_person(), a_person());
    let board = dashboard(&unique("placed"));
    owned(&store, &board, Owner::Person(owner)).await;
    let shared = Shared::Dashboard(&board);
    granted(&store, shared, Grantee::Person(viewer), Level::Viewer).await;
    let theirs = folder_of(&store, viewer).await;
    let as_viewer = Caller::administrator(viewer);

    let asked_to_carry = store
        .add_entry(theirs, &board, as_viewer, Entry::Carrying)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let carries_before = carries(&db, theirs, &board).await;
    let by_owner = store
        .add_entry(
            theirs,
            &board,
            Caller::administrator(owner),
            Entry::Carrying,
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let placed_by_owner = placer(&db, theirs, &board).await;
    let filed_again = store
        .add_entry(theirs, &board, as_viewer, Entry::Filing)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let placed_after = placer(&db, theirs, &board).await;

    remove(&store, &[&board]).await;
    remove_folders(&store, &[theirs]).await;
    assert_eq!(asked_to_carry, Entry::Filing);
    assert_eq!(carries_before, 0);
    assert_eq!(by_owner, Entry::Carrying);
    assert_eq!(placed_by_owner, Some(owner.to_string()));
    assert_eq!(filed_again, Entry::Carrying, "a filing never downgrades");
    assert_eq!(placed_after, Some(owner.to_string()));
}

#[tokio::test]
async fn an_owner_s_folders_stop_at_the_cap_however_creations_and_hand_overs_race() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (full, former, member) = (a_person(), a_person(), a_person());
    let mut made = Vec::with_capacity(MAX_FOLDERS);
    for _ in 1..MAX_FOLDERS {
        made.push(folder_of(&store, full).await);
    }
    let left = folder_of(&store, former).await;
    granted(
        &store,
        Shared::Folder(left),
        Grantee::Person(member),
        Level::Viewer,
    )
    .await;
    let name = || FolderName::parse("Raced").unwrap_or_else(|error| panic!("{error}"));

    let (first, second, handed) = tokio::join!(
        store.create_folder(full, name()),
        store.create_folder(full, name()),
        store.transfer_folder(left, former, full)
    );
    let handed = handed.unwrap_or_else(|error| panic!("{error}"));

    let created = [first, second];
    made.extend(
        created
            .iter()
            .filter_map(|made| made.as_ref().ok().map(|folder| folder.id)),
    );
    made.push(left);
    remove_folders(&store, &made).await;
    let landed = created.iter().filter(|made| made.is_ok()).count()
        + usize::from(handed == Handover::Handed);
    assert_eq!(landed, 1, "{created:?} {handed:?}");
    for refused in created.iter().filter(|made| made.is_err()) {
        assert!(matches!(refused, Err(FolderError::TooMany)), "{refused:?}");
    }
    assert!(
        matches!(handed, Handover::Handed | Handover::RecipientFull),
        "{handed:?}"
    );
}

#[tokio::test]
async fn a_grant_to_the_heir_racing_a_transfer_never_leaves_the_new_owner_a_grant() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let (former, editor, heir) = (a_person(), a_person(), a_person());
    let board = dashboard(&unique("heir"));
    owned(&store, &board, Owner::Person(former)).await;
    let shared = Shared::Dashboard(&board);
    granted(&store, shared, Grantee::Person(editor), Level::Editor).await;

    let (granting, handed) = tokio::join!(
        store.grant(shared, Grantee::Person(heir), Level::Viewer, editor),
        store.transfer(&board, former, heir)
    );
    let granting = granting.unwrap_or_else(|error| panic!("{error}"));
    let handed = handed.unwrap_or_else(|error| panic!("{error}"));
    let grants = store
        .grants_of(shared)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    remove(&store, &[&board]).await;
    assert_eq!(handed, Handover::Handed);
    assert!(
        matches!(granting, Granting::Done | Granting::ToOwner),
        "{granting:?}"
    );
    assert!(
        grants
            .iter()
            .all(|grant| grant.grantee != Grantee::Person(heir)),
        "{grants:?}"
    );
}

#[tokio::test]
async fn grants_on_neighbouring_dashboards_land_side_by_side() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let owner = a_person();
    let stem = unique("neighbour");
    let boards: Vec<DefinitionId> = (0..8)
        .map(|index| dashboard(&format!("{stem}_{index}")))
        .collect();
    for board in &boards {
        owned(&store, board, Owner::Person(owner)).await;
    }

    let granted_all = futures::future::join_all(boards.iter().map(|board| {
        store.grant(
            Shared::Dashboard(board),
            Grantee::Everyone,
            Level::Viewer,
            owner,
        )
    }))
    .await;

    remove(&store, &boards.iter().collect::<Vec<_>>()).await;
    for granting in granted_all {
        assert!(matches!(granting, Ok(Granting::Done)), "{granting:?}");
    }
}
