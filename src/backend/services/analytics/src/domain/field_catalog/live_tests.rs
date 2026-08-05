//! Drift test: the committed type snapshot (via the built catalog) must still
//! agree with the live ClickHouse schema. For every catalogued field, the
//! warehouse's column type — normalized — must equal the field's type in the
//! catalog. A mismatch means `types.snapshot.json` is stale and must be
//! regenerated from ClickHouse.
//!
//! Ignored by default; requires a live ClickHouse. Enable with
//! `INTEGRATION_TESTS_CLICKHOUSE_URL=<url> cargo test -p analytics -- --ignored
//! field_catalog`.

use std::collections::BTreeMap;

use clickhouse::Row;
use serde::Deserialize;

use crate::domain::field_catalog::field_catalog;
use crate::domain::field_catalog::model::FieldType;

const CH_ENV: &str = "INTEGRATION_TESTS_CLICKHOUSE_URL";
const CH_USER_ENV: &str = "INTEGRATION_TESTS_CLICKHOUSE_USER";
const CH_PASSWORD_ENV: &str = "INTEGRATION_TESTS_CLICKHOUSE_PASSWORD";

#[derive(Debug, Row, Deserialize)]
struct ColumnRow {
    name: String,
    #[serde(rename = "type")]
    ch_type: String,
}

#[tokio::test]
#[ignore = "requires live ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn types_snapshot_matches_clickhouse() {
    let Ok(url) = std::env::var(CH_ENV) else {
        eprintln!("skipping: {CH_ENV} not set");
        return;
    };
    let mut config = insight_clickhouse::Config::new(&url, "default");
    if let (Ok(user), Ok(password)) = (std::env::var(CH_USER_ENV), std::env::var(CH_PASSWORD_ENV)) {
        config = config.with_auth(user, password);
    }
    let client = insight_clickhouse::Client::new(config);

    let mut drift: Vec<String> = Vec::new();

    for dataset in &field_catalog().datasets {
        let rows = client
            .query("SELECT name, type FROM system.columns WHERE database = ? AND table = ?")
            .bind(dataset.database.as_str())
            .bind(dataset.table.as_str())
            .fetch_all::<ColumnRow>()
            .await
            .unwrap_or_else(|e| panic!("querying columns of {}: {e}", dataset.relation()));

        let live: BTreeMap<String, String> =
            rows.into_iter().map(|r| (r.name, r.ch_type)).collect();

        if live.is_empty() {
            drift.push(format!(
                "{}: relation absent from ClickHouse",
                dataset.relation()
            ));
            continue;
        }

        for field in &dataset.fields {
            match live.get(&field.name) {
                None => drift.push(format!(
                    "{}.{}: column absent from ClickHouse",
                    dataset.relation(),
                    field.name
                )),
                Some(ch_type) => match FieldType::normalize(ch_type) {
                    Some((ty, nullable)) if ty == field.ty && nullable == field.nullable => {}
                    other => drift.push(format!(
                        "{}.{}: catalog has {:?} (nullable={}), ClickHouse has {ch_type} -> {other:?}",
                        dataset.relation(),
                        field.name,
                        field.ty,
                        field.nullable
                    )),
                },
            }
        }
    }

    assert!(
        drift.is_empty(),
        "field-catalog type snapshot is stale — regenerate types.snapshot.json from ClickHouse:\n{}",
        drift.join("\n")
    );
}
