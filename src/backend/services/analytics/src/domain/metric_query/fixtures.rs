//! What the tests of every question in this family read against.

use uuid::Uuid;

use crate::domain::field_catalog::model::EntityType;

use super::catalog::product_metric_catalog;

pub const SHIPPED_METRIC: &str = "git.commits";
/// A shipped metric whose values are keyed by the tenant rather than a person.
pub const SHIPPED_TENANT_METRIC: &str = "ci.runs";
/// A shipped tenant metric whose computation is taken over per-row values.
pub const SHIPPED_TENANT_DISTRIBUTION_METRIC: &str = "ci.run_duration_min";
/// A shipped metric whose computation is taken over per-row values.
pub const SHIPPED_DISTRIBUTION_METRIC: &str = "git.pr_size";
/// A shipped metric composing two inputs, so a page must be told which to read.
pub const SHIPPED_RATIO_METRIC: &str = "git.merge_rate";

/// The subjects a question about a shipped metric names, at the grain that
/// metric records — so a sweep over every shipped metric asks each one a
/// question it can answer rather than one its grain refuses.
pub fn shipped_subjects(metric_key: &str) -> serde_json::Value {
    let catalog = product_metric_catalog().expect("the shipped definitions load");
    let metric = catalog
        .metric(metric_key)
        .unwrap_or_else(|| panic!("`{metric_key}` is shipped"));

    match metric.entity_type {
        EntityType::Person => serde_json::json!({
            "type": "persons",
            "ids": [Uuid::from_u128(1).to_string()],
        }),
        EntityType::Tenant => serde_json::json!({ "type": "tenant" }),
    }
}

/// Every shipped metric beside the parts of its computation a page may read.
pub fn shipped_input_roles() -> Vec<(&'static str, Vec<String>)> {
    let catalog = product_metric_catalog().expect("the shipped definitions load");

    catalog
        .metrics()
        .map(|metric| {
            let key = metric.key.as_str();
            let roles = catalog
                .input_roles(metric)
                .unwrap_or_else(|error| panic!("`{key}` resolves its inputs: {error}"));
            (key, roles)
        })
        .collect()
}

pub fn tenant() -> Uuid {
    Uuid::from_u128(0x7e_11a7)
}

/// Points at a closed port: a read reaching the network fails, never answers.
pub fn offline_clickhouse() -> insight_clickhouse::Client {
    insight_clickhouse::Client::new(insight_clickhouse::Config {
        url: "http://127.0.0.1:1".to_owned(),
        database: "insight".to_owned(),
        user: None,
        password: None,
        query_timeout: None,
        query_max_threads: None,
        query_max_memory_bytes: None,
    })
}
