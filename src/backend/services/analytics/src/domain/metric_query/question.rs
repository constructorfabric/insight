//! What every question in this family names, parsed once: a metric the
//! definitions carry, an inclusive window, the people it is about, and the
//! narrowing applied to every scan. INVARIANT: the ceilings live here alone,
//! so two endpoints cannot disagree about how much a caller may ask for.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use uuid::Uuid;

use crate::domain::compiler::request::DimensionFilter;

use super::catalog::MetricCatalog;
use super::dto::DimensionFilter as FilterDto;
use super::error::QueryError;

pub(super) const MAX_QUERIES: usize = 50;
pub(super) const MAX_SUBJECTS: usize = 1000;
pub(super) const MAX_WINDOW_DAYS: i64 = 400;
pub(super) const MAX_FILTERS: usize = 10;
pub(super) const MAX_FILTER_VALUES: usize = 100;
pub(super) const MAX_FILTER_VALUE_BYTES: usize = 512;
pub(super) const DATE_FORMAT: &str = "%Y-%m-%d";

/// Rows one question may report; the read binds one more, so exceeding the
/// ceiling is detected rather than silently truncated.
const ROW_LIMIT: usize = 5000;

#[must_use]
pub const fn row_limit() -> usize {
    ROW_LIMIT
}

#[must_use]
pub const fn query_row_limit() -> u64 {
    ROW_LIMIT as u64 + 1
}

/// How many questions one request may carry.
pub(super) fn batch_size(asked: usize) -> Result<(), QueryError> {
    if asked == 0 {
        return Err(QueryError::NoQueries);
    }
    if asked > MAX_QUERIES {
        return Err(QueryError::TooManyQueries { limit: MAX_QUERIES });
    }
    Ok(())
}

pub(super) fn defined_metric(catalog: &MetricCatalog, metric: &str) -> Result<String, QueryError> {
    let key = metric.trim();
    if catalog.metric(key).is_none() {
        return Err(QueryError::UnknownMetric {
            metric: key.to_owned(),
        });
    }
    Ok(key.to_owned())
}

/// The people a question names, parsed, deduplicated and ordered.
pub(super) fn person_ids(field: &'static str, ids: Vec<String>) -> Result<Vec<Uuid>, QueryError> {
    if ids.is_empty() {
        return Err(QueryError::NoSubjects { field });
    }
    if ids.len() > MAX_SUBJECTS {
        return Err(QueryError::TooManySubjects {
            field,
            limit: MAX_SUBJECTS,
        });
    }

    let parsed = ids
        .into_iter()
        .map(|id| {
            Uuid::parse_str(id.trim())
                .map_err(|_| QueryError::MalformedSubjectId { field, value: id })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(parsed.into_iter().collect())
}

pub(super) fn window(from: &str, to: &str) -> Result<(NaiveDate, NaiveDate), QueryError> {
    let from = date("queries.time.from", from)?;
    let to = date("queries.time.to", to)?;

    if from > to {
        return Err(QueryError::TimeReversed);
    }
    if (to - from).num_days() >= MAX_WINDOW_DAYS {
        return Err(QueryError::WindowTooLong {
            limit: MAX_WINDOW_DAYS,
        });
    }
    Ok((from, to))
}

pub(super) fn date(field: &'static str, value: &str) -> Result<NaiveDate, QueryError> {
    NaiveDate::parse_from_str(value.trim(), DATE_FORMAT).map_err(|_| QueryError::MalformedDate {
        field,
        value: value.to_owned(),
    })
}

/// INVARIANT: a filter narrows every measure the metric reads in one scan, so
/// it may only name a dimension every one of those measures declares.
pub(super) fn filters(
    catalog: &MetricCatalog,
    metric_key: &str,
    filters: Vec<FilterDto>,
) -> Result<Vec<DimensionFilter>, QueryError> {
    if filters.len() > MAX_FILTERS {
        return Err(QueryError::TooManyFilters { limit: MAX_FILTERS });
    }

    let declared = catalog.dimension_keys(metric_key);
    let mut seen = BTreeSet::new();
    let mut validated = Vec::with_capacity(filters.len());
    for filter in filters {
        let key = filter.dimension.trim().to_owned();
        if !declared.iter().any(|declared| *declared == key) {
            return Err(QueryError::UnknownFilterDimension { dimension: key });
        }
        if !seen.insert(key.clone()) {
            return Err(QueryError::DuplicateFilterDimension { dimension: key });
        }
        validated.push(DimensionFilter {
            values: filter_values(&key, filter.values)?,
            key,
        });
    }

    Ok(validated)
}

fn filter_values(dimension: &str, values: Vec<String>) -> Result<Vec<String>, QueryError> {
    if values.is_empty() {
        return Err(QueryError::NoFilterValues {
            dimension: dimension.to_owned(),
        });
    }
    if values.len() > MAX_FILTER_VALUES {
        return Err(QueryError::TooManyFilterValues {
            dimension: dimension.to_owned(),
            limit: MAX_FILTER_VALUES,
        });
    }
    if let Some(oversized) = values
        .iter()
        .find(|value| value.len() > MAX_FILTER_VALUE_BYTES)
    {
        return Err(QueryError::FilterValueTooLong {
            dimension: dimension.to_owned(),
            limit: MAX_FILTER_VALUE_BYTES,
            length: oversized.len(),
        });
    }

    Ok(values)
}

/// INVARIANT: the read binds one row over the ceiling, so an answer past it is
/// refused rather than served short.
pub(super) fn bounded<T>(rows: Vec<T>) -> Result<Vec<T>, QueryError> {
    if rows.len() > ROW_LIMIT {
        return Err(QueryError::ResultTooLarge { limit: ROW_LIMIT });
    }
    Ok(rows)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const FIELD: &str = "queries.subjects.ids";

    #[test]
    fn a_request_asks_between_one_question_and_the_ceiling() {
        assert!(matches!(
            batch_size(0).expect_err("a request asks something"),
            QueryError::NoQueries
        ));
        assert!(batch_size(1).is_ok());
        assert!(batch_size(MAX_QUERIES).is_ok());
        assert!(matches!(
            batch_size(MAX_QUERIES + 1).expect_err("one question over the ceiling is refused"),
            QueryError::TooManyQueries { .. }
        ));
    }

    #[test]
    fn the_people_a_question_names_are_parsed_once_and_ordered() {
        let second = Uuid::from_u128(2);
        let first = Uuid::from_u128(1);

        let parsed = person_ids(
            FIELD,
            vec![
                second.to_string(),
                first.to_string(),
                format!("  {first}  "),
            ],
        )
        .expect("three spellings of two people parse");

        assert_eq!(parsed, vec![first, second]);
    }

    #[test]
    fn a_person_reference_that_is_not_an_id_names_nobody() {
        assert!(matches!(
            person_ids(FIELD, vec!["nobody".to_owned()])
                .expect_err("a malformed reference is refused"),
            QueryError::MalformedSubjectId { .. }
        ));
    }

    #[test]
    fn a_window_runs_forward_and_no_longer_than_the_ceiling() {
        let cases = [
            ("2026-02-01", "2026-01-01", "a reversed window"),
            ("2026-01-01", "2027-06-01", "a window past the ceiling"),
            ("the first", "2026-01-31", "a date that is not one"),
        ];

        for (from, to, named) in cases {
            assert!(window(from, to).is_err(), "should refuse: {named}");
        }
        assert!(window("2026-01-01", "2026-01-31").is_ok());
    }

    #[test]
    fn an_answer_within_the_ceiling_is_served_and_one_past_it_is_refused() {
        assert_eq!(
            bounded(vec![0_u8; ROW_LIMIT]).map(|rows| rows.len()).ok(),
            Some(ROW_LIMIT)
        );

        assert!(matches!(
            bounded(vec![0_u8; ROW_LIMIT + 1]).expect_err("one row over the ceiling is refused"),
            QueryError::ResultTooLarge { .. }
        ));
    }
}
