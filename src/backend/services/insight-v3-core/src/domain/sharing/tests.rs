use std::sync::Mutex;

use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::domain::dashboards::DashboardCatalogue as _;
use crate::store::definitions::memory::MemoryDefinitions;

const FORMER: PersonId = PersonId::new(Uuid::from_u128(1));
const CALLER: PersonId = PersonId::new(Uuid::from_u128(2));
const RACER: PersonId = PersonId::new(Uuid::from_u128(3));

/// Identity as a list of administrators, which hands the dashboard to
/// someone else the moment its owner is looked up, as a second transfer
/// landing between the read and the write would.
struct Racing<'a> {
    store: &'a MemoryDefinitions,
    dashboard: DefinitionId,
    raced: Mutex<bool>,
}

#[async_trait]
impl Administrators for Racing<'_> {
    async fn holds_admin(&self, person: PersonId) -> Result<bool, AdministratorsError> {
        let first = std::mem::replace(
            &mut *self
                .raced
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            true,
        );
        if person == FORMER && !first {
            self.store
                .transfer(&self.dashboard, FORMER, RACER)
                .await
                .unwrap_or_else(|error| panic!("{error}"));
        }

        Ok(person != FORMER)
    }

    async fn current(&self) -> Result<Vec<PersonId>, AdministratorsError> {
        Ok(vec![CALLER, RACER])
    }
}

#[tokio::test]
async fn a_transfer_that_loses_the_race_is_refused_and_leaves_the_winner_owning_it() {
    let store = MemoryDefinitions::new();
    let dashboard = DefinitionId::parse("left_behind").unwrap_or_else(|error| panic!("{error}"));
    store
        .seed_owned(
            &dashboard,
            &json!({ "title": "Left", "widgets": [] }),
            FORMER,
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let administrators = Racing {
        store: &store,
        dashboard: dashboard.clone(),
        raced: Mutex::new(false),
    };

    let lost = DashboardShares::new(&store, &administrators, CALLER)
        .transfer(&dashboard, CALLER)
        .await;

    assert!(matches!(lost, Err(SharingError::Moved)), "{lost:?}");
    let standing = store
        .standing(&dashboard, CALLER)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(standing.map(|held| held.owner), Some(Owner::Person(RACER)));
}

/// Identity where only the people listed hold the administrator role.
struct Listed(Vec<PersonId>);

#[async_trait]
impl Administrators for Listed {
    async fn holds_admin(&self, person: PersonId) -> Result<bool, AdministratorsError> {
        Ok(self.0.contains(&person))
    }

    async fn current(&self) -> Result<Vec<PersonId>, AdministratorsError> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn a_transferred_dashboard_takes_its_tags_from_the_new_owners_scope() {
    use crate::domain::tags::{TagName, TagSet, Tags as _};

    let store = MemoryDefinitions::new();
    let handed = DefinitionId::parse("handed_over").unwrap_or_else(|error| panic!("{error}"));
    let mine = DefinitionId::parse("already_mine").unwrap_or_else(|error| panic!("{error}"));
    let body = json!({ "title": "Board", "widgets": [] });
    for (dashboard, owner, spelling) in [(&handed, FORMER, "finance"), (&mine, CALLER, "Finance")] {
        store
            .seed_owned(dashboard, &body, owner)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let tags = TagSet::parse(&[spelling.to_owned()]).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_tags(dashboard, Owner::Person(owner), &tags)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }

    DashboardShares::new(&store, &Listed(vec![CALLER]), CALLER)
        .transfer(&handed, CALLER)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let held = store
        .tags_of(&handed)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        held.iter().map(TagName::as_str).collect::<Vec<_>>(),
        ["Finance"],
        "the new owner's spelling should win once the tag is in their scope"
    );
}

/// Identity that fails every look at who holds the role.
struct Unreachable;

#[async_trait]
impl Administrators for Unreachable {
    async fn holds_admin(&self, _person: PersonId) -> Result<bool, AdministratorsError> {
        Err(AdministratorsError::Unavailable(
            "identity is down".to_owned(),
        ))
    }

    async fn current(&self) -> Result<Vec<PersonId>, AdministratorsError> {
        Err(AdministratorsError::Unavailable(
            "identity is down".to_owned(),
        ))
    }
}

async fn left_by_former(store: &MemoryDefinitions, id: &str) -> DefinitionId {
    let dashboard = DefinitionId::parse(id).unwrap_or_else(|error| panic!("{error}"));
    store
        .seed_owned(&dashboard, &json!({ "title": id, "widgets": [] }), FORMER)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    dashboard
}

#[tokio::test]
async fn an_identity_failure_is_answered_as_missing_only_where_the_caller_cannot_see_it() {
    let store = MemoryDefinitions::new();
    let unseen = left_by_former(&store, "unseen").await;
    let seen = left_by_former(&store, "seen").await;
    store
        .share(&seen, CALLER, FORMER)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let shares = DashboardShares::new(&store, &Unreachable, CALLER);

    let hidden = shares.transfer(&unseen, CALLER).await;
    let failed = shares.transfer(&seen, CALLER).await;

    assert!(
        matches!(&hidden, Err(SharingError::NotFound(id)) if id == "unseen"),
        "{hidden:?}"
    );
    assert!(
        matches!(failed, Err(SharingError::Administrators(_))),
        "{failed:?}"
    );
}

#[tokio::test]
async fn an_identity_failure_refuses_a_share_and_the_orphaned_listing() {
    let store = MemoryDefinitions::new();
    let mine = DefinitionId::parse("mine").unwrap_or_else(|error| panic!("{error}"));
    store
        .seed_owned(&mine, &json!({ "title": "Mine", "widgets": [] }), CALLER)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let shares = DashboardShares::new(&store, &Unreachable, CALLER);

    let granted = shares.grant(&mine, RACER).await;
    let page = Page::parse(None, None).unwrap_or_else(|error| panic!("{error}"));
    let orphaned = shares.orphaned(page).await;

    assert!(
        matches!(granted, Err(SharingError::Administrators(_))),
        "{granted:?}"
    );
    assert!(
        matches!(orphaned, Err(SharingError::Administrators(_))),
        "{orphaned:?}"
    );
    let held = store
        .shares_of(&mine)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(held.is_empty(), "{held:?}");
}

#[tokio::test]
async fn a_transfer_overfilling_the_new_owners_tags_is_refused_and_leaves_the_owner() {
    use crate::domain::tags::{MAX_TAGS_PER_DASHBOARD, TagSet, Tags as _};

    let store = MemoryDefinitions::new();
    let body = json!({ "title": "Board", "widgets": [] });
    for board in 0..MAX_TAGS / MAX_TAGS_PER_DASHBOARD {
        let id =
            DefinitionId::parse(&format!("full_{board}")).unwrap_or_else(|error| panic!("{error}"));
        store
            .seed_owned(&id, &body, CALLER)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let names: Vec<String> = (0..MAX_TAGS_PER_DASHBOARD)
            .map(|tag| format!("tag {board} {tag}"))
            .collect();
        let tags = TagSet::parse(&names).unwrap_or_else(|error| panic!("{error}"));
        store
            .set_tags(&id, Owner::Person(CALLER), &tags)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
    let handed = left_by_former(&store, "handed").await;
    let extra = TagSet::parse(&["one more".to_owned()]).unwrap_or_else(|error| panic!("{error}"));
    store
        .set_tags(&handed, Owner::Person(FORMER), &extra)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let refused = DashboardShares::new(&store, &Listed(vec![CALLER]), CALLER)
        .transfer(&handed, CALLER)
        .await;

    assert!(
        matches!(refused, Err(SharingError::RecipientTagsFull)),
        "{refused:?}"
    );
    let standing = store
        .standing(&handed, CALLER)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(standing.map(|held| held.owner), Some(Owner::Person(FORMER)));
}
