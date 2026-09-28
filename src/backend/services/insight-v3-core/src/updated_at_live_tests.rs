use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::domain::definition::{Change, DefinitionKind, DefinitionName, Definitions};
use crate::live_mariadb::{store_or_skip, unique};
use crate::store::definitions::MariaDefinitions;

fn dashboard(name: &str) -> DefinitionName {
    DefinitionName::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

async fn stamp(store: &MariaDefinitions, name: &DefinitionName) -> Option<DateTime<Utc>> {
    store
        .updated_at(DefinitionKind::Dashboard, name)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
}

#[tokio::test]
async fn a_write_is_stamped_in_utc_and_a_later_write_moves_the_stamp() {
    let Some(store) = store_or_skip().await else {
        return;
    };
    let name = dashboard(&unique("stamped"));
    let before = Utc::now() - chrono::Duration::seconds(1);

    store
        .put(DefinitionKind::Dashboard, &name, &json!({ "title": "A" }))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let first = stamp(&store, &name).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    store
        .apply(&[Change::Put(
            DefinitionKind::Dashboard,
            name.clone(),
            json!({ "title": "B" }),
        )])
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let second = stamp(&store, &name).await;
    let after = Utc::now() + chrono::Duration::seconds(1);

    Definitions::delete(&store, DefinitionKind::Dashboard, &name)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let gone = stamp(&store, &name).await;
    assert!(
        first.is_some_and(|stamp| before <= stamp && stamp <= after),
        "{first:?} should fall between {before} and {after}"
    );
    assert!(second > first, "{first:?} then {second:?}");
    assert!(second.is_some_and(|stamp| stamp <= after), "{second:?}");
    assert_eq!(gone, None);
}
