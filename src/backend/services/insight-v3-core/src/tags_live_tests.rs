use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::domain::definition::{Change, DefinitionKind, DefinitionName, Definitions, Page};
use crate::domain::folders::{FolderFilter, FolderName, Folders};
use crate::domain::tags::{MAX_TAGS, TagError, TagFilter, TagSet, Tags};
use crate::live_mariadb::{store_or_skip, unique};
use crate::store::definitions::MariaDefinitions;

static ONE_AT_A_TIME: Mutex<()> = Mutex::const_new(());

fn dashboard(name: &str) -> DefinitionName {
    DefinitionName::parse(name).unwrap_or_else(|error| panic!("{error}"))
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

async fn board(store: &MariaDefinitions, name: &DefinitionName) {
    store
        .put(
            DefinitionKind::Dashboard,
            name,
            &json!({ "title": "Board", "items": [] }),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

async fn remove(store: &MariaDefinitions, name: &DefinitionName) {
    Definitions::delete(store, DefinitionKind::Dashboard, name)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

async fn set(store: &MariaDefinitions, name: &DefinitionName, values: &[String]) {
    store
        .set_tags(name, &tags(values))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

async fn carried(store: &MariaDefinitions, name: &DefinitionName) -> Vec<String> {
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
        .list_tags()
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .find(|summary| summary.name.as_str() == tag)
        .map(|summary| summary.dashboards)
}

async fn total(store: &MariaDefinitions) -> usize {
    store
        .list_tags()
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
async fn tags_survive_a_body_write_follow_a_rename_and_go_with_the_dashboard() {
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
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                after.clone(),
                json!({ "title": "Board", "items": [] }),
            ),
            Change::CarryFolder {
                from: before.clone(),
                to: after.clone(),
            },
            Change::CarryTags {
                from: before.clone(),
                to: after.clone(),
            },
            Change::Delete(DefinitionKind::Dashboard, before.clone()),
        ])
        .await
        .unwrap_or_else(|error| panic!("{error}"));
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
    let found = store
        .page_tagged(&format!("{prefix}{needle}"), page, folder, &any_of(wanted))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let names = found
        .names
        .iter()
        .map(|name| name.trim_start_matches(prefix).to_owned())
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
    let platform = store
        .create_folder(
            FolderName::parse(&unique("Platform")).unwrap_or_else(|error| panic!("{error}")),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    for filed in [named("delivery"), named("hiring")] {
        store
            .file(&filed, Some(platform.id))
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
        Some(FolderFilter::In(platform.id)),
        &either,
    )
    .await;
    let unfiled = paged(&store, &prefix, "", Some(FolderFilter::Unfiled), &either).await;

    for (name, _) in &boards {
        remove(&store, name).await;
    }
    store
        .delete_folder(platform.id)
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
        .set_tags(&dashboard(&unique("nowhere")), &tags(&[short()]))
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
