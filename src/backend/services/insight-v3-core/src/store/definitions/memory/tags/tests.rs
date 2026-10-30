use std::error::Error;

use serde_json::json;

use crate::domain::access::{Caller, Owner, PersonId};
use crate::domain::dashboards::{DashboardCatalogue as _, DashboardPage, DashboardQuery};
use crate::domain::definition::{Change, DefinitionId, DefinitionKind, Definitions, Page};
use crate::domain::folders::FolderFilter;
use crate::domain::tags::{MAX_TAGS, TagError, TagFilter, TagSet, Tags};
use crate::store::definitions::memory::MemoryDefinitions;

type R = Result<(), Box<dyn Error>>;

fn name(value: &str) -> DefinitionId {
    DefinitionId::parse(value).unwrap_or_else(|error| panic!("the name must parse: {error}"))
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

fn ids(page: &DashboardPage) -> Vec<&str> {
    page.items.iter().map(|listed| listed.id.as_str()).collect()
}

fn me() -> PersonId {
    PersonId::new(uuid::Uuid::from_u128(1))
}

fn someone_else() -> PersonId {
    PersonId::new(uuid::Uuid::from_u128(2))
}

/// The scope an ownerless board's tags are named in.
const OWNERLESS: Owner = Owner::Unowned;

fn tagged(needle: &str, folder: Option<FolderFilter>, filter: TagFilter) -> DashboardQuery {
    DashboardQuery {
        needle: needle.to_owned(),
        folder,
        tags: Some(filter),
        ..DashboardQuery::default()
    }
}

async fn with_boards(names: &[&str]) -> MemoryDefinitions {
    let store = MemoryDefinitions::new();
    for board in names {
        store
            .seed(
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
        .list_tags(Caller::administrator(me()))
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
        .set_tags(
            &name("delivery"),
            OWNERLESS,
            &tags(&["Platform", "Delivery"]),
        )
        .await?;
    store
        .set_tags(&name("support"), OWNERLESS, &tags(&["Platform"]))
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
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["Delivery"]))
        .await?;

    store
        .set_tags(&name("support"), OWNERLESS, &tags(&["delivery"]))
        .await?;

    assert_eq!(carried(&store, "support").await, ["Delivery"]);
    assert_eq!(listed(&store).await, [("Delivery".to_owned(), 2)]);

    Ok(())
}

#[tokio::test]
async fn re_tagging_the_only_carrier_in_another_case_keeps_the_spelling() -> R {
    let store = with_boards(&["delivery"]).await;
    store
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["Delivery"]))
        .await?;

    store
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["DELIVERY"]))
        .await?;

    assert_eq!(carried(&store, "delivery").await, ["Delivery"]);

    Ok(())
}

#[tokio::test]
async fn setting_the_tags_replaces_the_whole_set() -> R {
    let store = with_boards(&["delivery"]).await;
    store
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["Alpha", "Beta"]))
        .await?;

    store
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["Beta", "Gamma"]))
        .await?;

    assert_eq!(carried(&store, "delivery").await, ["Beta", "Gamma"]);

    Ok(())
}

#[tokio::test]
async fn a_dashboard_s_tags_read_back_sorted_ignoring_case() -> R {
    let store = with_boards(&["delivery"]).await;

    store
        .set_tags(
            &name("delivery"),
            OWNERLESS,
            &tags(&["beta", "Alpha", "gamma"]),
        )
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
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["Old", "Kept"]))
        .await?;
    store
        .set_tags(&name("support"), OWNERLESS, &tags(&["Cleared"]))
        .await?;

    store
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["Kept", "New"]))
        .await?;
    store
        .set_tags(&name("support"), OWNERLESS, &tags(&[]))
        .await?;

    assert_eq!(
        listed(&store).await,
        [("Kept".to_owned(), 1), ("New".to_owned(), 1)]
    );

    Ok(())
}

#[tokio::test]
async fn tagging_a_dashboard_that_does_not_exist_is_refused() {
    let store = MemoryDefinitions::new();

    let refused = store
        .set_tags(&name("nowhere"), OWNERLESS, &tags(&["Delivery"]))
        .await;

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
        store
            .set_tags(&name(board), OWNERLESS, &TagSet::parse(&full)?)
            .await?;
    }
    let last = &boards[MAX_TAGS / 10];
    store
        .set_tags(&name(last), OWNERLESS, &tags(&["t0_0"]))
        .await?;

    let refused = store
        .set_tags(&name(last), OWNERLESS, &tags(&["t0_0", "one more"]))
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
        store
            .set_tags(&name(board), OWNERLESS, &TagSet::parse(&full)?)
            .await?;
    }
    let swapped: Vec<String> = (1..10)
        .map(|tag| format!("t0_{tag}"))
        .chain(["fresh".to_owned()])
        .collect();

    store
        .set_tags(&name("b0"), OWNERLESS, &TagSet::parse(&swapped)?)
        .await?;

    assert!(carried(&store, "b0").await.contains(&"fresh".to_owned()));
    assert_eq!(listed(&store).await.len(), MAX_TAGS);

    Ok(())
}

#[tokio::test]
async fn a_body_write_leaves_the_dashboard_s_tags() -> R {
    let store = with_boards(&["delivery"]).await;
    store
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["Delivery"]))
        .await?;

    store
        .seed(
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
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["Delivery", "Shared"]))
        .await?;
    store
        .set_tags(&name("support"), OWNERLESS, &tags(&["Shared"]))
        .await?;

    Definitions::delete(&store, DefinitionKind::Dashboard, &name("delivery")).await?;

    assert_eq!(listed(&store).await, [("Shared".to_owned(), 1)]);
    assert!(carried(&store, "delivery").await.is_empty());

    Ok(())
}

#[tokio::test]
async fn carried_tags_go_to_the_copy_in_the_same_batch() -> R {
    let store = with_boards(&["original"]).await;
    store
        .set_tags(
            &name("original"),
            OWNERLESS,
            &tags(&["Delivery", "Platform"]),
        )
        .await?;

    store
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                name("copy"),
                crate::domain::definition::DefinitionLabel::parse("Copy")?,
                json!({ "title": "original" }),
            ),
            Change::CarryTags {
                from: name("original"),
                to: name("copy"),
                scope: OWNERLESS,
            },
        ])
        .await?;

    assert_eq!(carried(&store, "copy").await, ["Delivery", "Platform"]);
    assert_eq!(
        listed(&store).await,
        [("Delivery".to_owned(), 2), ("Platform".to_owned(), 2)]
    );

    Ok(())
}

#[tokio::test]
async fn a_page_by_tag_holds_the_dashboards_carrying_any_of_them() -> R {
    let store = with_boards(&["delivery", "delivery_ai", "hiring", "support"]).await;
    store
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["Alpha"]))
        .await?;
    store
        .set_tags(&name("delivery_ai"), OWNERLESS, &tags(&["Alpha", "Beta"]))
        .await?;
    store
        .set_tags(&name("hiring"), OWNERLESS, &tags(&["Beta"]))
        .await?;
    store
        .set_tags(&name("support"), OWNERLESS, &tags(&["Gamma"]))
        .await?;
    let page = Page::parse(None, None)?;

    let found = store
        .page_dashboards(
            Caller::administrator(me()),
            &tagged("", None, any_of(&["alpha", "BETA"])),
            page,
        )
        .await?;

    assert_eq!(ids(&found), ["delivery", "delivery_ai", "hiring"]);
    assert_eq!(found.total, 3);

    Ok(())
}

#[tokio::test]
async fn a_page_by_tag_still_searches_and_keeps_to_its_folder() -> R {
    let store = with_boards(&["delivery", "delivery_ai", "hiring"]).await;
    for board in ["delivery", "delivery_ai", "hiring"] {
        store
            .set_tags(&name(board), OWNERLESS, &tags(&["Alpha"]))
            .await?;
    }
    let platform = store.seed_ownerless_folder("Platform");
    store.seed_entry(platform, &name("delivery"));
    store.seed_entry(platform, &name("hiring"));
    let page = Page::parse(None, None)?;
    let alpha = any_of(&["Alpha"]);

    let searched = store
        .page_dashboards(
            Caller::administrator(me()),
            &tagged("deliv", None, alpha.clone()),
            page,
        )
        .await?;
    let filed = store
        .page_dashboards(
            Caller::administrator(me()),
            &tagged("", Some(FolderFilter::In(platform)), alpha.clone()),
            page,
        )
        .await?;
    let unfiled = store
        .page_dashboards(
            Caller::administrator(me()),
            &tagged("", Some(FolderFilter::Unfiled), alpha),
            page,
        )
        .await?;

    assert_eq!(ids(&searched), ["delivery", "delivery_ai"]);
    assert_eq!(ids(&filed), ["delivery", "hiring"]);
    assert_eq!(ids(&unfiled), ["delivery_ai"]);

    Ok(())
}

#[tokio::test]
async fn a_page_by_a_tag_nobody_carries_is_empty() -> R {
    let store = with_boards(&["delivery"]).await;
    store
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["Alpha"]))
        .await?;

    let found = store
        .page_dashboards(
            Caller::administrator(me()),
            &tagged("", None, any_of(&["Omega"])),
            Page::parse(None, None)?,
        )
        .await?;

    assert!(ids(&found).is_empty());
    assert_eq!(found.total, 0);

    Ok(())
}

#[tokio::test]
async fn a_store_that_is_down_refuses_to_tag() {
    let store = MemoryDefinitions::refusing();

    let refused = store
        .set_tags(&name("delivery"), OWNERLESS, &tags(&["Alpha"]))
        .await;

    assert!(matches!(refused, Err(TagError::Store(_))), "{refused:?}");
}

async fn owned_by(store: &MemoryDefinitions, board: &str, owner: PersonId) -> R {
    store
        .apply(&[Change::Own {
            dashboard: name(board),
            owner,
        }])
        .await?;

    Ok(())
}

#[tokio::test]
async fn two_owners_hold_tags_of_one_name_apart_and_a_filter_finds_both() -> R {
    let store = with_boards(&["mine", "theirs"]).await;
    owned_by(&store, "mine", me()).await?;
    owned_by(&store, "theirs", someone_else()).await?;
    crate::domain::sharing::Grants::grant(
        &store,
        crate::domain::sharing::Shared::Dashboard(&name("theirs")),
        crate::domain::access::Grantee::Person(me()),
        crate::domain::access::Level::Viewer,
        someone_else(),
    )
    .await?;

    store
        .set_tags(&name("mine"), Owner::Person(me()), &tags(&["Delivery"]))
        .await?;
    store
        .set_tags(
            &name("theirs"),
            Owner::Person(someone_else()),
            &tags(&["delivery"]),
        )
        .await?;

    assert_eq!(carried(&store, "mine").await, ["Delivery"]);
    assert_eq!(
        carried(&store, "theirs").await,
        ["delivery"],
        "each scope keeps its spelling"
    );
    assert_eq!(listed(&store).await, [("Delivery".to_owned(), 2)]);
    let found = store
        .page_dashboards(
            Caller::administrator(me()),
            &tagged("", None, any_of(&["DELIVERY"])),
            Page::parse(None, None)?,
        )
        .await?;
    assert_eq!(ids(&found), ["mine", "theirs"]);

    Ok(())
}

#[tokio::test]
async fn tags_on_a_dashboard_the_caller_cannot_see_are_not_listed_or_counted() -> R {
    let store = with_boards(&["mine", "private"]).await;
    owned_by(&store, "mine", me()).await?;
    owned_by(&store, "private", someone_else()).await?;
    store
        .set_tags(&name("mine"), Owner::Person(me()), &tags(&["Shared"]))
        .await?;
    store
        .set_tags(
            &name("private"),
            Owner::Person(someone_else()),
            &tags(&["Shared", "Secret"]),
        )
        .await?;

    assert_eq!(listed(&store).await, [("Shared".to_owned(), 1)]);

    Ok(())
}

#[tokio::test]
async fn the_tag_cap_is_per_owner() -> R {
    let boards: Vec<String> = (0..=MAX_TAGS / 10)
        .map(|index| format!("b{index}"))
        .collect();
    let names: Vec<&str> = boards.iter().map(String::as_str).collect();
    let store = with_boards(&names).await;
    for (index, board) in boards.iter().take(MAX_TAGS / 10).enumerate() {
        let full: Vec<String> = (0..10).map(|tag| format!("t{index}_{tag}")).collect();
        store
            .set_tags(&name(board), Owner::Person(me()), &TagSet::parse(&full)?)
            .await?;
    }
    let last = &boards[MAX_TAGS / 10];

    let theirs = store
        .set_tags(
            &name(last),
            Owner::Person(someone_else()),
            &tags(&["one more"]),
        )
        .await;
    let mine = store
        .set_tags(&name(last), Owner::Person(me()), &tags(&["one more"]))
        .await;

    assert!(theirs.is_ok(), "{theirs:?}");
    assert!(matches!(mine, Err(TagError::TooMany)), "{mine:?}");

    Ok(())
}
