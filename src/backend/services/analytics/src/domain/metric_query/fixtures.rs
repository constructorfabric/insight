//! What the tests of every question in this family read against.

use uuid::Uuid;

pub const SHIPPED_METRIC: &str = "git.commits";
/// A shipped metric whose computation is taken over per-row values.
pub const SHIPPED_DISTRIBUTION_METRIC: &str = "git.pr_size";

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
