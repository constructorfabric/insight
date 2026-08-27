//! The boundary between what a caller wrote and what a distribution reasons
//! about: a metric taken over per-row values, the people it is about, and the
//! two readings of the shape a question may ask for. INVARIANT: a question
//! asking for neither is answered with the default histogram, never nothing.

use std::num::NonZeroU32;

use chrono::NaiveDate;
use uuid::Uuid;

use crate::domain::compiler::request::DimensionFilter;

use super::super::catalog::MetricCatalog;
use super::super::dto::Subjects;
use super::super::error::QueryError;
use super::super::question::{batch_size, defined_metric, filters, person_ids, window};
use super::dto::{DistributionQuery, DistributionsRequest};

/// The field a distribution question names its people in.
const SUBJECTS_FIELD: &str = "queries.subjects.ids";

/// The view name a refusal about the metric's computation is reported under.
const VIEW: &str = "distributions";

/// Bins a question that names none is answered with.
const DEFAULT_BINS: u32 = 10;
const MAX_BINS: u32 = 100;
const MAX_QUANTILES: usize = 10;

#[derive(Debug, PartialEq)]
pub struct ValidatedDistribution {
    pub metric_key: String,
    pub subjects: Vec<Uuid>,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub filters: Vec<DimensionFilter>,
    /// Absent when the question asked only for quantiles.
    pub bins: Option<NonZeroU32>,
    /// Absent when the question named none; otherwise sorted and deduplicated.
    pub quantiles: Option<Vec<f64>>,
}

#[derive(Debug, PartialEq)]
pub struct ValidatedDistributions {
    pub queries: Vec<ValidatedDistribution>,
}

impl ValidatedDistributions {
    /// Every person the request asks about, deduplicated across its questions.
    #[must_use]
    pub fn subject_ids(&self) -> Vec<Uuid> {
        self.queries
            .iter()
            .flat_map(|query| query.subjects.iter().copied())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

pub fn validate_request(
    catalog: &MetricCatalog,
    request: DistributionsRequest,
) -> Result<ValidatedDistributions, QueryError> {
    batch_size(request.queries.len())?;

    let queries = request
        .queries
        .into_iter()
        .map(|query| validate_query(catalog, query))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ValidatedDistributions { queries })
}

fn validate_query(
    catalog: &MetricCatalog,
    query: DistributionQuery,
) -> Result<ValidatedDistribution, QueryError> {
    let metric_key = defined_metric(catalog, &query.metric)?;
    distributable(catalog, &metric_key)?;
    let subjects = subjects(query.subjects)?;
    let (from, to) = window(&query.time.from, &query.time.to)?;
    let filters = filters(catalog, &metric_key, query.filters)?;
    let quantiles = query.quantiles.map(quantiles).transpose()?;
    let bins = bins(query.bins, quantiles.is_some())?;

    Ok(ValidatedDistribution {
        metric_key,
        subjects,
        from,
        to,
        filters,
        bins,
        quantiles,
    })
}

/// INVARIANT: the rule is the compiler's own — only a computation taken over
/// the measure's per-row values has a distribution — so a question refused
/// here would have been refused there, and one admitted here compiles.
fn distributable(catalog: &MetricCatalog, metric_key: &str) -> Result<(), QueryError> {
    let Some(metric) = catalog.metric(metric_key) else {
        return Err(QueryError::UnknownMetric {
            metric: metric_key.to_owned(),
        });
    };

    catalog
        .distribution(metric, VIEW)
        .map_err(QueryError::NoDistribution)
}

/// A distribution reports the values a subject's own events carried, and a
/// dataset records no event keyed by the tenant.
fn subjects(subjects: Subjects) -> Result<Vec<Uuid>, QueryError> {
    match subjects {
        Subjects::Persons { ids } => person_ids(SUBJECTS_FIELD, ids),
        Subjects::Tenant {} => Err(QueryError::Unanswerable {
            reason: "a distribution reports the values of a subject's own events, and no event \
                     is recorded against the tenant itself",
        }),
    }
}

/// How the range is cut. A question naming quantiles alone asks for no
/// histogram, so none is read.
fn bins(asked: Option<u32>, asks_quantiles: bool) -> Result<Option<NonZeroU32>, QueryError> {
    let count = match asked {
        Some(count) => count,
        None if asks_quantiles => return Ok(None),
        None => DEFAULT_BINS,
    };

    let Some(bins) = NonZeroU32::new(count).filter(|bins| bins.get() <= MAX_BINS) else {
        return Err(QueryError::BinsOutOfRange { limit: MAX_BINS });
    };
    Ok(Some(bins))
}

/// The positions a question asks for, ordered and asked for once each.
fn quantiles(asked: Vec<f64>) -> Result<Vec<f64>, QueryError> {
    if asked.is_empty() {
        return Err(QueryError::NoQuantiles);
    }
    if asked.len() > MAX_QUANTILES {
        return Err(QueryError::TooManyQuantiles {
            limit: MAX_QUANTILES,
        });
    }
    if let Some(outside) = asked
        .iter()
        .find(|quantile| !(quantile.is_finite() && **quantile > 0.0 && **quantile < 1.0))
    {
        return Err(QueryError::QuantileOutOfRange { quantile: *outside });
    }

    let mut sorted = asked;
    // SAFETY: every position is finite and inside `(0, 1)`, so the comparison
    // is total and no ordering is left undefined.
    sorted.sort_by(f64::total_cmp);
    sorted.dedup();
    Ok(sorted)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::super::catalog::product_metric_catalog;
    use super::super::super::fixtures::{SHIPPED_DISTRIBUTION_METRIC, SHIPPED_METRIC};
    use super::*;

    fn catalog() -> &'static MetricCatalog {
        product_metric_catalog().expect("the shipped definitions load")
    }

    fn person() -> Uuid {
        Uuid::from_u128(1)
    }

    fn query(overrides: &serde_json::Value) -> serde_json::Value {
        let mut query = serde_json::json!({
            "metric": SHIPPED_DISTRIBUTION_METRIC,
            "subjects": { "type": "persons", "ids": [person().to_string()] },
            "time": { "from": "2026-01-01", "to": "2026-01-31" },
        });
        let base = query.as_object_mut().expect("an object");
        for (key, value) in overrides.as_object().expect("an object") {
            base.insert(key.clone(), value.clone());
        }
        query
    }

    fn validate(query: &serde_json::Value) -> Result<ValidatedDistributions, QueryError> {
        let request: DistributionsRequest =
            serde_json::from_value(serde_json::json!({ "queries": [query] }))
                .expect("the wire shape parses");
        validate_request(catalog(), request)
    }

    #[test]
    fn a_question_naming_neither_reading_is_answered_with_the_default_histogram() {
        let validated = validate(&query(&serde_json::json!({}))).expect("a shipped metric answers");

        assert_eq!(validated.queries[0].bins, NonZeroU32::new(DEFAULT_BINS));
        assert_eq!(validated.queries[0].quantiles, None);
    }

    #[test]
    fn a_question_naming_quantiles_alone_reads_no_histogram() {
        let validated = validate(&query(&serde_json::json!({ "quantiles": [0.5] })))
            .expect("quantiles alone are answerable");

        assert_eq!(validated.queries[0].bins, None);
        assert_eq!(validated.queries[0].quantiles, Some(vec![0.5]));
    }

    #[test]
    fn a_question_naming_both_readings_reads_both() {
        let validated = validate(&query(&serde_json::json!({
            "bins": 4,
            "quantiles": [0.9, 0.5, 0.5],
        })))
        .expect("both readings are answerable");

        assert_eq!(validated.queries[0].bins, NonZeroU32::new(4));
        assert_eq!(
            validated.queries[0].quantiles,
            Some(vec![0.5, 0.9]),
            "the positions are ordered and asked for once each"
        );
    }

    #[test]
    fn a_bin_count_outside_the_range_a_histogram_is_cut_into_is_refused() {
        let cases = [
            (serde_json::json!(0), "no bin at all"),
            (serde_json::json!(MAX_BINS + 1), "one bin past the ceiling"),
        ];

        for (count, named) in cases {
            let outcome = validate(&query(&serde_json::json!({ "bins": count })));

            assert!(
                matches!(outcome, Err(QueryError::BinsOutOfRange { .. })),
                "should refuse: {named}"
            );
        }
        assert!(validate(&query(&serde_json::json!({ "bins": 1 }))).is_ok());
        assert!(validate(&query(&serde_json::json!({ "bins": MAX_BINS }))).is_ok());
    }

    #[test]
    fn a_quantile_list_is_refused_when_it_is_empty_out_of_range_or_past_the_cap() {
        let cases = [
            (serde_json::json!([]), "a list naming no position"),
            (serde_json::json!([0.0]), "the zeroth position"),
            (serde_json::json!([1.0]), "the whole distribution"),
            (serde_json::json!([-0.5]), "a position below zero"),
            (serde_json::json!([1.5]), "a position above one"),
            (
                serde_json::json!((1..=11).map(|n| f64::from(n) / 20.0).collect::<Vec<_>>()),
                "one position past the ceiling",
            ),
        ];

        for (quantiles, named) in cases {
            let outcome = validate(&query(&serde_json::json!({ "quantiles": quantiles })));

            assert!(outcome.is_err(), "should refuse: {named}");
        }
    }

    #[test]
    fn only_a_metric_whose_computation_ranks_per_row_values_has_a_distribution() {
        let refused = validate(&query(&serde_json::json!({ "metric": SHIPPED_METRIC })))
            .expect_err("a counted metric has no per-event values");

        assert!(matches!(refused, QueryError::NoDistribution(_)));
        assert!(
            refused.to_string().contains("percentile or stddev"),
            "the refusal names the rule: {refused}"
        );
    }

    #[test]
    fn a_tenant_has_no_events_of_its_own_to_take_a_distribution_of() {
        let refused = validate(&query(&serde_json::json!({
            "subjects": { "type": "tenant" },
        })))
        .expect_err("nothing keys an event by the tenant");

        assert!(matches!(refused, QueryError::Unanswerable { .. }));
    }

    #[test]
    fn a_subject_list_is_refused_when_it_is_empty_or_unreadable() {
        for ids in [serde_json::json!([]), serde_json::json!(["nobody"])] {
            let named = ids.to_string();
            let outcome = validate(&query(&serde_json::json!({
                "subjects": { "type": "persons", "ids": ids },
            })));

            assert!(outcome.is_err(), "should refuse: {named}");
        }
    }

    #[test]
    fn the_same_subject_named_twice_is_read_once() {
        let validated = validate(&query(&serde_json::json!({
            "subjects": { "type": "persons", "ids": [person().to_string(), person().to_string()] },
        })))
        .expect("a repeated subject is one subject");

        assert_eq!(validated.queries[0].subjects, vec![person()]);
        assert_eq!(validated.subject_ids(), vec![person()]);
    }
}
