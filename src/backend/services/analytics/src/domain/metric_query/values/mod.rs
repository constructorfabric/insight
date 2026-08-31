//! What a metric's value is, for the people and window a question names: one
//! number per subject, or one series per bucket, optionally broken out by a
//! dimension and set beside an earlier window's.

mod answerable;
mod assemble;
mod dto;
mod group_cap;
mod plan;
mod service;
mod validation;

pub use dto::{CompareOffset, Fold, Grain, ValuesRequest, ValuesResponse};
pub use service::answer;
pub use validation::{ValidatedBatch, validate_request};

pub(super) use answerable::{offered_compare_offsets, offered_folds, offered_grains};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod fixtures {
    use chrono::NaiveDate;
    use uuid::Uuid;

    use crate::domain::field_catalog::model::EntityType;

    use super::super::fixtures::SHIPPED_METRIC;
    use super::dto::Grain;
    use super::validation::{QueryShape, ValidatedQuery, ValidatedSplit, ValidatedSubjects};

    pub fn validated(shape: QueryShape, grain: Grain, dimensions: &[&str]) -> ValidatedQuery {
        ValidatedQuery {
            metric_key: SHIPPED_METRIC.to_owned(),
            entity_type: EntityType::Person,
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
