//! What the tests of every question in this family read against.

use uuid::Uuid;

use super::catalog::product_metric_catalog;

pub const SHIPPED_METRIC: &str = "git.commits";
/// A shipped metric whose computation is taken over per-row values.
pub const SHIPPED_DISTRIBUTION_METRIC: &str = "git.pr_size";
/// A shipped metric composing two inputs, so a page must be told which to read.
pub const SHIPPED_RATIO_METRIC: &str = "git.merge_rate";

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
