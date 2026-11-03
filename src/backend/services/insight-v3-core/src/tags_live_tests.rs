use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::domain::access::{Caller, Grantee, Level, Owner, PersonId};
use crate::domain::dashboards::{DashboardCatalogue as _, DashboardQuery};
use crate::domain::definition::{
    Change, DefinitionId, DefinitionKind, DefinitionLabel, Definitions, Page,
};
use crate::domain::folders::{FolderFilter, Folders};
use crate::domain::tags::{MAX_TAGS, TagError, TagFilter, TagSet, Tags};
use crate::live_mariadb::{store_or_skip, unique};
use crate::store::definitions::MariaDefinitions;

pub(crate) static ONE_AT_A_TIME: Mutex<()> = Mutex::const_new(());

/// Someone who owns nothing here, so sees exactly the ownerless dashboards.
fn anyone() -> PersonId {
    PersonId::new(Uuid::now_v7())
}

fn admin(person: PersonId) -> Caller {
    Caller::administrator(person)
}

fn dashboard(name: &str) -> DefinitionId {
    DefinitionId::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

fn short() -> String {
    let id = Uuid::now_v7().simple().to_string();

    id[id.len() - 12..].to_owned()
}

fn tags(values: &[String]) -> TagSet {
    TagSet::parse(values).unwrap_or_else(|error| panic!("{error}"))
}

fn any_of(values: &[String]) -> TagFilter {
    TagFilter::parse(DefinitionKind::Dashboard, values)
        .unwrap_or_else(|error| panic!("{error}"))
        .unwrap_or_else(|| panic!("a filter naming tags is one"))
}

/// Stores the board, or writes its body again when it is already there.
async fn board(store: &MariaDefinitions, name: &DefinitionId) {
    let label = DefinitionLabel::of_id(name);
    let body = json!({ "title": "Board", "items": [] });

    let replaced = store
        .replace(DefinitionKind::Dashboard, name, &label, &body)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    if !replaced {
        store
            .create(DefinitionKind::Dashboard, name, &label, &body)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

fn copied(from: &DefinitionId, to: &DefinitionId) -> [Change; 2] {
    [
        Change::Create(
            DefinitionKind::Dashboard,
            to.clone(),
            DefinitionLabel::of_id(to),
            json!({ "title": "Board", "items": [] }),
        ),
        Change::CarryTags {
            from: from.clone(),
            to: to.clone(),
            scope: Owner::Unowned,
        },
    ]
}

async fn remove(store: &MariaDefinitions, name: &DefinitionId) {
    Definitions::delete(store, DefinitionKind::Dashboard, name)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

async fn set(store: &MariaDefinitions, name: &DefinitionId, values: &[String]) {
    store
        .set_tags(name, Owner::Unowned, &tags(values))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

/// A folder no one owns, as one from before ownership is, which is where
/// ownerless boards go.
async fn ownerless_folder(
    store: &MariaDefinitions,
    name: &str,
) -> crate::domain::folders::FolderId {
    use sea_orm::ConnectionTrait as _;

    let id = crate::domain::folders::FolderId::new();
    store
        .connection()
        .execute_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DbBackend::MySql,
            "INSERT INTO folders (id, name, owner_person_id, created_at, updated_at)
VALUES (?, ?, NULL, UTC_TIMESTAMP(6), UTC_TIMESTAMP(6))",
            [id.to_string().into(), name.into()],
        ))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    id
}

async fn carried(store: &MariaDefinitions, name: &DefinitionId) -> Vec<String> {
    store
        .tags_of(name)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .map(|tag| tag.as_str().to_owned())
        .collect()
}

async fn count_of(store: &MariaDefinitions, tag: &str) -> Option<u64> {
    store
        .list_tags(admin(anyone()))
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .find(|summary| summary.name.as_str() == tag)
        .map(|summary| summary.dashboards)
}

async fn total(store: &MariaDefinitions) -> usize {
    store
        .list_tags(admin(anyone()))
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .len()
}

#[tokio::test]
async fn a_tag_is_reused_under_its_spelling_and_goes_when_no_dashboard_carries_it() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let suffix = short();
    let delivery = format!("Delivery {suffix}");
    let old = format!("Old {suffix}");
    let first = dashboard(&unique("first"));
    let second = dashboard(&unique("second"));
    board(&store, &first).await;
    board(&store, &second).await;

    set(&store, &first, &[delivery.clone(), old.clone()]).await;
    set(&store, &second, &[delivery.to_lowercase()]).await;
    assert_eq!(carried(&store, &second).await, [delivery.as_str()]);
    assert_eq!(count_of(&store, &delivery).await, Some(2));

    set(&store, &first, &[delivery.to_uppercase()]).await;
    assert_eq!(carried(&store, &first).await, [delivery.as_str()]);
    assert_eq!(count_of(&store, &old).await, None, "an orphan must go");

    remove(&store, &first).await;
    remove(&store, &second).await;
    assert_eq!(count_of(&store, &delivery).await, None);
}

#[tokio::test]
async fn tags_survive_a_body_write_follow_a_copy_and_go_with_the_dashboard() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let suffix = short();
    let alpha = format!("Alpha {suffix}");
    let beta = format!("beta {suffix}");
    let before = dashboard(&unique("before"));
    let after = dashboard(&unique("after"));
    board(&store, &before).await;
    set(&store, &before, &[beta.clone(), alpha.clone()]).await;

    board(&store, &before).await;
    assert_eq!(
        carried(&store, &before).await,
        [alpha.clone(), beta.clone()],
        "a body write must leave the tags alone, sorted ignoring case"
    );

    store
        .apply(&copied(&before, &after))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(count_of(&store, &alpha).await, Some(2));
    remove(&store, &before).await;
    assert_eq!(carried(&store, &after).await, [alpha.clone(), beta.clone()]);
    assert!(carried(&store, &before).await.is_empty());
    assert_eq!(count_of(&store, &alpha).await, Some(1));

    remove(&store, &after).await;
    assert!(carried(&store, &after).await.is_empty());
    assert_eq!(count_of(&store, &alpha).await, None);
    assert_eq!(count_of(&store, &beta).await, None);
}

async fn paged(
    store: &MariaDefinitions,
    prefix: &str,
    needle: &str,
    folder: Option<FolderFilter>,
    wanted: &[String],
) -> (Vec<String>, u64) {
    let page = Page::parse(None, None).unwrap_or_else(|error| panic!("{error}"));
    let query = DashboardQuery {
        needle: format!("{prefix}{needle}"),
        folder,
        tags: Some(any_of(wanted)),
        ..DashboardQuery::default()
    };
    let found = store
        .page_dashboards(admin(anyone()), &query, page)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let names = found
        .items
        .iter()
        .map(|listed| listed.id.trim_start_matches(prefix).to_owned())
        .collect();
    (names, found.total)
}

#[tokio::test]
async fn a_page_by_tag_matches_any_of_them_ignoring_case_inside_a_folder_and_a_search() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let suffix = short();
    let alpha = format!("Alpha {suffix}");
    let beta = format!("Beta {suffix}");
    let prefix = format!("{}_", unique("paged"));
    let named = |rest: &str| dashboard(&format!("{prefix}{rest}"));
    let boards = [
        (named("delivery"), vec![alpha.clone()]),
        (named("delivery_ai"), vec![alpha.clone(), beta.clone()]),
        (named("hiring"), vec![beta.clone()]),
        (named("support"), vec![format!("Gamma {suffix}")]),
    ];
    for (name, carrying) in &boards {
        board(&store, name).await;
        set(&store, name, carrying).await;
    }
    let platform = ownerless_folder(&store, &unique("Platform")).await;
    for filed in [named("delivery"), named("hiring")] {
        crate::domain::folders::add(&store, admin(anyone()), platform, &filed)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
    let either = [alpha.to_lowercase(), beta.to_uppercase()];

    let everywhere = paged(&store, &prefix, "", None, &either).await;
    let searched = paged(&store, &prefix, "deliv", None, std::slice::from_ref(&alpha)).await;
    let filed = paged(
        &store,
        &prefix,
        "",
        Some(FolderFilter::In(platform)),
        &either,
    )
    .await;
    let unfiled = paged(&store, &prefix, "", Some(FolderFilter::Unfiled), &either).await;

    for (name, _) in &boards {
        remove(&store, name).await;
    }
    store
        .delete_folder(platform)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(everywhere.0, ["delivery", "delivery_ai", "hiring"]);
    assert_eq!(everywhere.1, 3);
    assert_eq!(searched.0, ["delivery", "delivery_ai"]);
    assert_eq!(filed.0, ["delivery", "hiring"]);
    assert_eq!(unfiled.0, ["delivery_ai"]);
}

#[tokio::test]
async fn tagging_a_dashboard_that_is_not_there_is_refused() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;

    let refused = store
        .set_tags(
            &dashboard(&unique("nowhere")),
            Owner::Unowned,
            &tags(&[short()]),
        )
        .await;

    assert!(
        matches!(refused, Err(TagError::DashboardNotFound(_))),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_set_that_would_take_the_total_past_the_cap_is_refused_and_changes_nothing() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let held = total(&store).await;
    let suffix = short();
    let fresh: Vec<String> = (0..MAX_TAGS - held)
        .map(|index| format!("cap {index} {suffix}"))
        .collect();
    let mut boards = Vec::new();
    for chunk in fresh.chunks(10) {
        let name = dashboard(&unique("full"));
        board(&store, &name).await;
        set(&store, &name, chunk).await;
        boards.push(name);
    }
    let last = dashboard(&unique("last"));
    board(&store, &last).await;
    boards.push(last.clone());
    set(&store, &last, &fresh[..1]).await;

    let refused = store
        .set_tags(
            &last,
            Owner::Unowned,
            &tags(&[fresh[0].clone(), format!("one more {suffix}")]),
        )
        .await;
    let kept = carried(&store, &last).await;
    let at_the_cap = total(&store).await;

    for name in &boards {
        remove(&store, name).await;
    }
    assert!(matches!(refused, Err(TagError::TooMany)), "{refused:?}");
    assert_eq!(kept, fresh[..1]);
    assert_eq!(at_the_cap, MAX_TAGS);
    assert_eq!(total(&store).await, held);
}

#[tokio::test]
async fn tag_names_differing_by_accent_or_emoji_are_different_tags() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let suffix = short();
    let name = dashboard(&unique("accents"));
    board(&store, &name).await;
    let spelled = [
        format!("Café {suffix}"),
        format!("Cafe {suffix}"),
        format!("🚀 {suffix}"),
        format!("🔥 {suffix}"),
    ];

    set(&store, &name, &spelled).await;
    let held = carried(&store, &name).await;

    remove(&store, &name).await;
    assert_eq!(held.len(), 4, "{held:?}");
}

async fn tag_set_copied_and_removed(
    store: &MariaDefinitions,
    from: &DefinitionId,
    wanted: &TagSet,
) -> Result<(), String> {
    let to = dashboard(&unique("raced"));

    store
        .set_tags(from, Owner::Unowned, wanted)
        .await
        .map_err(|error| format!("set: {error:?}"))?;
    store
        .apply(&copied(from, &to))
        .await
        .map_err(|error| format!("copy: {error:?}"))?;
    for held in [from, &to] {
        Definitions::delete(store, DefinitionKind::Dashboard, held)
            .await
            .map_err(|error| format!("delete: {error:?}"))?;
    }

    Ok(())
}

#[tokio::test]
async fn tag_writers_running_at_once_all_land() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let store = std::sync::Arc::new(store);
    let suffix = short();
    let boards: Vec<DefinitionId> = (0..16)
        .map(|index| dashboard(&unique(&format!("racing{index}"))))
        .collect();
    for name in &boards {
        board(&store, name).await;
    }

    let writers: Vec<_> = boards
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let store = std::sync::Arc::clone(&store);
            let name = name.clone();
            let wanted = tags(&[format!("race {index} {suffix}"), format!("shared {suffix}")]);
            tokio::spawn(async move { tag_set_copied_and_removed(&store, &name, &wanted).await })
        })
        .collect();
    let mut failures = Vec::new();
    for writer in writers {
        if let Err(failure) = writer.await.unwrap_or_else(|error| panic!("{error}")) {
            failures.push(failure);
        }
    }

    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(count_of(&store, &format!("shared {suffix}")).await, None);
}

async fn owned_board(store: &MariaDefinitions, name: &DefinitionId, owner: PersonId) {
    store
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                name.clone(),
                DefinitionLabel::of_id(name),
                json!({ "title": "Board", "items": [] }),
            ),
            Change::Own {
                dashboard: name.clone(),
                owner,
            },
        ])
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

#[tokio::test]
async fn two_owners_hold_one_tag_name_apart_and_a_listing_merges_what_the_caller_sees() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let _serial = ONE_AT_A_TIME.lock().await;
    let suffix = short();
    let delivery = format!("Delivery {suffix}");
    let (me, them) = (anyone(), anyone());
    let mine = dashboard(&unique("mine"));
    let theirs = dashboard(&unique("theirs"));
    owned_board(&store, &mine, me).await;
    owned_board(&store, &theirs, them).await;

    store
        .set_tags(
            &mine,
            Owner::Person(me),
            &tags(std::slice::from_ref(&delivery)),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    store
        .set_tags(
            &theirs,
            Owner::Person(them),
            &tags(&[delivery.to_lowercase()]),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let before_sharing = store
        .list_tags(admin(me))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    crate::domain::sharing::Grants::grant(
        &store,
        crate::domain::sharing::Shared::Dashboard(&theirs),
        Grantee::Person(me),
        Level::Viewer,
        them,
    )
    .await
    .unwrap_or_else(|error| panic!("{error}"));
    let shared = store
        .list_tags(admin(me))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let page = Page::parse(None, None).unwrap_or_else(|error| panic!("{error}"));
    let found = store
        .page_dashboards(
            admin(me),
            &DashboardQuery {
                tags: Some(any_of(&[delivery.to_uppercase()])),
                ..DashboardQuery::default()
            },
            page,
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let theirs_spelled = carried(&store, &theirs).await;
    remove(&store, &mine).await;
    remove(&store, &theirs).await;
    let count = |listed: &[crate::domain::tags::TagSummary]| {
        listed
            .iter()
            .find(|summary| summary.name.folded() == delivery.to_lowercase())
            .map(|summary| summary.dashboards)
    };
    assert_eq!(
        theirs_spelled,
        [delivery.to_lowercase()],
        "each scope keeps its spelling"
    );
    assert_eq!(count(&before_sharing), Some(1));
    assert_eq!(count(&shared), Some(2));
    assert_eq!(found.total, 2);
}
