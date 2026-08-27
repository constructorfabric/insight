//! Answering questions about metric values from the semantic definitions:
//! parse the question into typed values, decide which read answers it, resolve
//! what that read needs, run it, and state the answer in explicit fields. A
//! combination no dataset can answer is named and refused, never approximated.

mod assemble;
mod catalog;
pub(crate) mod dto;
mod error;
mod execute;
mod group_cap;
mod plan;
mod provenance;
mod service;
mod validation;

pub use catalog::product_metric_catalog;
pub use dto::{ValuesRequest, ValuesResponse};
pub use service::answer;
pub use validation::{ValidatedBatch, validate_request};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod fixtures {
    use chrono::NaiveDate;
    use uuid::Uuid;

    use super::dto::Grain;
    use super::validation::{QueryShape, ValidatedQuery, ValidatedSplit, ValidatedSubjects};

    pub const SHIPPED_METRIC: &str = "git.commits";

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

    pub fn validated(shape: QueryShape, grain: Grain, dimensions: &[&str]) -> ValidatedQuery {
        ValidatedQuery {
            metric_key: SHIPPED_METRIC.to_owned(),
            subjects: ValidatedSubjects::Persons(vec![Uuid::from_u128(1)]),
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
            grain,
            filters: Vec::new(),
            split: (!dimensions.is_empty()).then(|| ValidatedSplit {
                dimensions: dimensions.iter().map(|key| (*key).to_owned()).collect(),
                limit: None,
            }),
            compare: None,
            shape,
        }
    }
}
