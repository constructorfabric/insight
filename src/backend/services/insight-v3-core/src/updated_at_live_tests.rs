use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::domain::definition::{
    Change, DefinitionId, DefinitionKind, DefinitionLabel, Definitions,
};
use crate::live_mariadb::{store_or_skip, unique};
use crate::store::definitions::MariaDefinitions;

fn dashboard(name: &str) -> DefinitionId {
    DefinitionId::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

async fn stamp(store: &MariaDefinitions, id: &DefinitionId) -> Option<DateTime<Utc>> {
    store
        .read(DefinitionKind::Dashboard, id)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .map(|stored| stored.updated_at)
}

#[tokio::test]
async fn a_write_is_stamped_in_utc_and_a_later_write_moves_the_stamp() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let id = dashboard(&unique("stamped"));
    let label = DefinitionLabel::of_id(&id);
    let before = Utc::now() - chrono::Duration::seconds(1);

    store
        .create(
            DefinitionKind::Dashboard,
            &id,
            &label,
            &json!({ "title": "A" }),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let first = stamp(&store, &id).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    store
        .apply(&[Change::Replace(
            DefinitionKind::Dashboard,
            id.clone(),
            label,
            json!({ "title": "B" }),
        )])
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let second = stamp(&store, &id).await;
    let after = Utc::now() + chrono::Duration::seconds(1);

    Definitions::delete(&store, DefinitionKind::Dashboard, &id)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let gone = stamp(&store, &id).await;
    assert!(
        first.is_some_and(|stamp| before <= stamp && stamp <= after),
        "{first:?} should fall between {before} and {after}"
    );
    assert!(second > first, "{first:?} then {second:?}");
    assert!(second.is_some_and(|stamp| stamp <= after), "{second:?}");
    assert_eq!(gone, None);
}

#[tokio::test]
async fn a_replace_of_an_id_nobody_holds_is_refused_and_writes_nothing() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let id = dashboard(&unique("missing"));
    let created = dashboard(&unique("alongside"));

    let refused = store
        .apply(&[
            Change::Create(
                DefinitionKind::Dashboard,
                created.clone(),
                DefinitionLabel::of_id(&created),
                json!({ "title": "A" }),
            ),
            Change::Replace(
                DefinitionKind::Dashboard,
                id.clone(),
                DefinitionLabel::of_id(&id),
                json!({ "title": "B" }),
            ),
        ])
        .await;

    assert!(
        matches!(
            &refused,
            Err(crate::domain::definition::DefinitionStoreError::Missing(missing)) if missing == id.as_str()
        ),
        "{refused:?}"
    );
    assert_eq!(stamp(&store, &created).await, None);
}
