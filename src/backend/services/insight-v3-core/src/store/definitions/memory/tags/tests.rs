use std::error::Error;

use serde_json::json;

use crate::domain::definition::{Change, DefinitionKind, DefinitionName, Definitions, Page};
use crate::domain::folders::{FolderFilter, FolderName, Folders};
use crate::domain::tags::{MAX_TAGS, TagError, TagFilter, TagSet, Tags};
use crate::store::definitions::memory::MemoryDefinitions;

type R = Result<(), Box<dyn Error>>;

fn name(value: &str) -> DefinitionName {
    DefinitionName::parse(value).unwrap_or_else(|error| panic!("the name must parse: {error}"))
}

fn tags(values: &[&str]) -> TagSet {
    let owned: Vec<String> = values.iter().map(|value| (*value).to_owned()).collect();

    TagSet::parse(&owned).unwrap_or_else(|error| panic!("the tags must parse: {error}"))
}

fn any_of(values: &[&str]) -> TagFilter {
    let owned: Vec<String> = values.iter().map(|value| (*value).to_owned()).collect();

    TagFilter::parse(DefinitionKind::Dashboard, &owned)
        .unwrap_or_else(|error| panic!("the filter must parse: {error}"))
        .unwrap_or_else(|| panic!("a filter naming tags is one"))
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

async fn listed(store: &MemoryDefinitions) -> Vec<(String, u64)> {
    store
        .list_tags()
        .await
        .unwrap_or_else(|error| panic!("the tags must list: {error}"))
        .into_iter()
        .map(|summary| (summary.name.as_str().to_owned(), summary.dashboards))
        .collect()
}

async fn carried(store: &MemoryDefinitions, dashboard: &str) -> Vec<String> {
    store
        .tags_of(&name(dashboard))
        .await
        .unwrap_or_else(|error| panic!("the tags of `{dashboard}` must read: {error}"))
        .into_iter()
        .map(|tag| tag.as_str().to_owned())
        .collect()
}

#[tokio::test]
async fn a_tag_is_listed_with_how_many_dashboards_carry_it() -> R {
    let store = with_boards(&["delivery", "support"]).await;

    store
        .set_tags(&name("delivery"), &tags(&["Platform", "Delivery"]))
        .await?;
    store
        .set_tags(&name("support"), &tags(&["Platform"]))
        .await?;

    assert_eq!(
        listed(&store).await,
        [("Delivery".to_owned(), 1), ("Platform".to_owned(), 2)]
    );

    Ok(())
}

#[tokio::test]
async fn an_existing_tag_is_reused_under_its_stored_spelling() -> R {
    let store = with_boards(&["delivery", "support"]).await;
    store
        .set_tags(&name("delivery"), &tags(&["Delivery"]))
        .await?;

    store
        .set_tags(&name("support"), &tags(&["delivery"]))
        .await?;

    assert_eq!(carried(&store, "support").await, ["Delivery"]);
    assert_eq!(listed(&store).await, [("Delivery".to_owned(), 2)]);

    Ok(())
}

#[tokio::test]
async fn re_tagging_the_only_carrier_in_another_case_keeps_the_spelling() -> R {
    let store = with_boards(&["delivery"]).await;
    store
        .set_tags(&name("delivery"), &tags(&["Delivery"]))
        .await?;

    store
        .set_tags(&name("delivery"), &tags(&["DELIVERY"]))
        .await?;

    assert_eq!(carried(&store, "delivery").await, ["Delivery"]);

    Ok(())
}

#[tokio::test]
async fn setting_the_tags_replaces_the_whole_set() -> R {
    let store = with_boards(&["delivery"]).await;
    store
        .set_tags(&name("delivery"), &tags(&["Alpha", "Beta"]))
        .await?;

    store
        .set_tags(&name("delivery"), &tags(&["Beta", "Gamma"]))
        .await?;

    assert_eq!(carried(&store, "delivery").await, ["Beta", "Gamma"]);

    Ok(())
}

#[tokio::test]
async fn a_dashboard_s_tags_read_back_sorted_ignoring_case() -> R {
    let store = with_boards(&["delivery"]).await;

    store
        .set_tags(&name("delivery"), &tags(&["beta", "Alpha", "gamma"]))
        .await?;

    assert_eq!(
        carried(&store, "delivery").await,
        ["Alpha", "beta", "gamma"]
    );

    Ok(())
}

#[tokio::test]
async fn a_tag_no_dashboard_carries_any_more_is_gone() -> R {
    let store = with_boards(&["delivery", "support"]).await;
    store
        .set_tags(&name("delivery"), &tags(&["Old", "Kept"]))
        .await?;
    store
        .set_tags(&name("support"), &tags(&["Cleared"]))
        .await?;

    store
        .set_tags(&name("delivery"), &tags(&["Kept", "New"]))
        .await?;
    store.set_tags(&name("support"), &tags(&[])).await?;

    assert_eq!(
        listed(&store).await,
        [("Kept".to_owned(), 1), ("New".to_owned(), 1)]
    );

    Ok(())
}

#[tokio::test]
async fn tagging_a_dashboard_that_does_not_exist_is_refused() {
    let store = MemoryDefinitions::new();

    let refused = store.set_tags(&name("nowhere"), &tags(&["Delivery"])).await;

    assert!(
        matches!(refused, Err(TagError::DashboardNotFound(_))),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_set_that_would_take_the_total_past_the_cap_is_refused_and_changes_nothing() -> R {
    let boards: Vec<String> = (0..=MAX_TAGS / 10)
        .map(|index| format!("b{index}"))
        .collect();
    let names: Vec<&str> = boards.iter().map(String::as_str).collect();
    let store = with_boards(&names).await;
    for (index, board) in boards.iter().take(MAX_TAGS / 10).enumerate() {
        let full: Vec<String> = (0..10).map(|tag| format!("t{index}_{tag}")).collect();
        store.set_tags(&name(board), &TagSet::parse(&full)?).await?;
    }
    let last = &boards[MAX_TAGS / 10];
    store.set_tags(&name(last), &tags(&["t0_0"])).await?;

    let refused = store
        .set_tags(&name(last), &tags(&["t0_0", "one more"]))
        .await;

    assert!(matches!(refused, Err(TagError::TooMany)), "{refused:?}");
    assert_eq!(carried(&store, last).await, ["t0_0"]);
    assert_eq!(listed(&store).await.len(), MAX_TAGS);

    Ok(())
}

#[tokio::test]
async fn at_the_cap_a_tag_may_still_be_swapped_for_a_new_one() -> R {
    let boards: Vec<String> = (0..MAX_TAGS / 10)
        .map(|index| format!("b{index}"))
        .collect();
    let names: Vec<&str> = boards.iter().map(String::as_str).collect();
    let store = with_boards(&names).await;
    for (index, board) in boards.iter().enumerate() {
        let full: Vec<String> = (0..10).map(|tag| format!("t{index}_{tag}")).collect();
        store.set_tags(&name(board), &TagSet::parse(&full)?).await?;
    }
    let swapped: Vec<String> = (1..10)
        .map(|tag| format!("t0_{tag}"))
        .chain(["fresh".to_owned()])
        .collect();

    store
        .set_tags(&name("b0"), &TagSet::parse(&swapped)?)
        .await?;

    assert!(carried(&store, "b0").await.contains(&"fresh".to_owned()));
    assert_eq!(listed(&store).await.len(), MAX_TAGS);

    Ok(())
}

#[tokio::test]
async fn a_body_write_leaves_the_dashboard_s_tags() -> R {
    let store = with_boards(&["delivery"]).await;
    store
        .set_tags(&name("delivery"), &tags(&["Delivery"]))
        .await?;

    store
        .put(
            DefinitionKind::Dashboard,
            &name("delivery"),
            &json!({ "title": "Rewritten" }),
        )
        .await?;

    assert_eq!(carried(&store, "delivery").await, ["Delivery"]);

    Ok(())
}

#[tokio::test]
async fn deleting_a_dashboard_drops_its_tags_and_those_only_it_carried() -> R {
    let store = with_boards(&["delivery", "support"]).await;
    store
        .set_tags(&name("delivery"), &tags(&["Delivery", "Shared"]))
        .await?;
    store.set_tags(&name("support"), &tags(&["Shared"])).await?;

    Definitions::delete(&store, DefinitionKind::Dashboard, &name("delivery")).await?;

    assert_eq!(listed(&store).await, [("Shared".to_owned(), 1)]);
    assert!(carried(&store, "delivery").await.is_empty());

    Ok(())
}

#[tokio::test]
async fn carried_tags_move_with_the_name_in_the_same_batch() -> R {
    let store = with_boards(&["old"]).await;
    store
        .set_tags(&name("old"), &tags(&["Delivery", "Platform"]))
        .await?;

    store
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                name("new"),
                json!({ "title": "old" }),
            ),
            Change::CarryTags {
                from: name("old"),
                to: name("new"),
            },
            Change::Delete(DefinitionKind::Dashboard, name("old")),
        ])
        .await?;

    assert_eq!(carried(&store, "new").await, ["Delivery", "Platform"]);
    assert_eq!(
        listed(&store).await,
        [("Delivery".to_owned(), 1), ("Platform".to_owned(), 1)]
    );

    Ok(())
}

#[tokio::test]
async fn a_page_by_tag_holds_the_dashboards_carrying_any_of_them() -> R {
    let store = with_boards(&["delivery", "delivery_ai", "hiring", "support"]).await;
    store.set_tags(&name("delivery"), &tags(&["Alpha"])).await?;
    store
        .set_tags(&name("delivery_ai"), &tags(&["Alpha", "Beta"]))
        .await?;
    store.set_tags(&name("hiring"), &tags(&["Beta"])).await?;
    store.set_tags(&name("support"), &tags(&["Gamma"])).await?;
    let page = Page::parse(None, None)?;

    let found = store
        .page_tagged("", page, None, &any_of(&["alpha", "BETA"]))
        .await?;

    assert_eq!(found.names, ["delivery", "delivery_ai", "hiring"]);
    assert_eq!(found.total, 3);

    Ok(())
}

#[tokio::test]
async fn a_page_by_tag_still_searches_and_keeps_to_its_folder() -> R {
    let store = with_boards(&["delivery", "delivery_ai", "hiring"]).await;
    for board in ["delivery", "delivery_ai", "hiring"] {
        store.set_tags(&name(board), &tags(&["Alpha"])).await?;
    }
    let platform = store.create_folder(FolderName::parse("Platform")?).await?;
    store.file(&name("delivery"), Some(platform.id)).await?;
    store.file(&name("hiring"), Some(platform.id)).await?;
    let page = Page::parse(None, None)?;
    let alpha = any_of(&["Alpha"]);

    let searched = store.page_tagged("deliv", page, None, &alpha).await?;
    let filed = store
        .page_tagged("", page, Some(FolderFilter::In(platform.id)), &alpha)
        .await?;
    let unfiled = store
        .page_tagged("", page, Some(FolderFilter::Unfiled), &alpha)
        .await?;

    assert_eq!(searched.names, ["delivery", "delivery_ai"]);
    assert_eq!(filed.names, ["delivery", "hiring"]);
    assert_eq!(unfiled.names, ["delivery_ai"]);

    Ok(())
}

#[tokio::test]
async fn a_page_by_a_tag_nobody_carries_is_empty() -> R {
    let store = with_boards(&["delivery"]).await;
    store.set_tags(&name("delivery"), &tags(&["Alpha"])).await?;

    let found = store
        .page_tagged("", Page::parse(None, None)?, None, &any_of(&["Omega"]))
        .await?;

    assert!(found.names.is_empty());
    assert_eq!(found.total, 0);

    Ok(())
}

#[tokio::test]
async fn a_store_that_is_down_refuses_to_tag() {
    let store = MemoryDefinitions::refusing();

    let refused = store.set_tags(&name("delivery"), &tags(&["Alpha"])).await;

    assert!(matches!(refused, Err(TagError::Store(_))), "{refused:?}");
}
