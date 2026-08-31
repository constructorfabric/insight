//! The boundary between what a caller wrote and what this service reasons
//! about: parsed dates, parsed person ids, a metric key the definitions carry,
//! and one [`QueryShape`]. INVARIANT: a combination the semantic layer cannot
//! answer never becomes a `ValidatedQuery`.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use uuid::Uuid;

use super::catalog::MetricCatalog;
use super::dto::{Fold, Grain, Split, SplitLimit, Subjects, ValuesQuery, ValuesRequest};
use super::error::QueryError;

const MAX_QUERIES: usize = 50;
const MAX_SUBJECTS: usize = 1000;
const MAX_WINDOW_DAYS: i64 = 400;
const MAX_SPLIT_DIMENSIONS: usize = 10;
const MAX_SPLIT_TOP: u32 = 50;
const DATE_FORMAT: &str = "%Y-%m-%d";

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

/// Which read answers one question, decided here and nowhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryShape {
    /// One value per subject over the whole window.
    SubjectTotal,
    /// One value per subject per split group, over the whole window.
    SubjectSplit,
    /// One value per split group, folded over every subject.
    CombinedSplit,
    /// One series per subject and split group, plus the window total.
    SubjectSeries,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ValidatedSubjects {
    Persons(Vec<Uuid>),
    Tenant,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ValidatedSplitLimit {
    pub top: u32,
    /// The read's own metric unless the question named another to rank by.
    pub rank_by: String,
    pub remainder: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ValidatedSplit {
    pub dimensions: Vec<String>,
    pub limit: Option<ValidatedSplitLimit>,
}

#[derive(Debug, PartialEq)]
pub struct ValidatedQuery {
    pub metric_key: String,
    pub subjects: ValidatedSubjects,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub grain: Grain,
    pub split: Option<ValidatedSplit>,
    pub shape: QueryShape,
}

impl ValidatedQuery {
    /// The dimensions the groups are keyed by, in the order their columns take.
    pub fn dimensions(&self) -> &[String] {
        self.split
            .as_ref()
            .map_or(&[], |split| split.dimensions.as_slice())
    }
}

#[derive(Debug, PartialEq)]
pub struct ValidatedBatch {
    pub queries: Vec<ValidatedQuery>,
}

impl ValidatedBatch {
    /// Every person the request asks about, deduplicated across its questions.
    #[must_use]
    pub fn subject_ids(&self) -> Vec<Uuid> {
        self.queries
            .iter()
            .filter_map(|query| match &query.subjects {
                ValidatedSubjects::Persons(ids) => Some(ids.iter().copied()),
                ValidatedSubjects::Tenant => None,
            })
            .flatten()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[must_use]
    pub fn asks_about_the_tenant(&self) -> bool {
        self.queries
            .iter()
            .any(|query| matches!(query.subjects, ValidatedSubjects::Tenant))
    }
}

pub fn validate_request(
    catalog: &MetricCatalog,
    request: ValuesRequest,
) -> Result<ValidatedBatch, QueryError> {
    if request.queries.is_empty() {
        return Err(QueryError::NoQueries);
    }
    if request.queries.len() > MAX_QUERIES {
        return Err(QueryError::TooManyQueries { limit: MAX_QUERIES });
    }

    let queries = request
        .queries
        .into_iter()
        .map(|query| validate_query(catalog, query))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ValidatedBatch { queries })
}

fn validate_query(
    catalog: &MetricCatalog,
    query: ValuesQuery,
) -> Result<ValidatedQuery, QueryError> {
    let metric_key = defined_metric(catalog, &query.metric)?;
    let subjects = subjects(query.subjects)?;
    let (from, to) = window(&query.time.from, &query.time.to)?;
    let split = query
        .split
        .map(|split| validate_split(catalog, &metric_key, split))
        .transpose()?;
    let shape = shape(query.time.grain, query.fold, split.as_ref(), &subjects)?;

    Ok(ValidatedQuery {
        metric_key,
        subjects,
        from,
        to,
        grain: query.time.grain,
        split,
        shape,
    })
}

fn defined_metric(catalog: &MetricCatalog, metric: &str) -> Result<String, QueryError> {
    let key = metric.trim();
    if catalog.metric(key).is_none() {
        return Err(QueryError::UnknownMetric {
            metric: key.to_owned(),
        });
    }
    Ok(key.to_owned())
}

fn subjects(subjects: Subjects) -> Result<ValidatedSubjects, QueryError> {
    match subjects {
        Subjects::Tenant {} => Ok(ValidatedSubjects::Tenant),
        Subjects::Persons { ids } => {
            if ids.is_empty() {
                return Err(QueryError::NoSubjects);
            }
            if ids.len() > MAX_SUBJECTS {
                return Err(QueryError::TooManySubjects {
                    limit: MAX_SUBJECTS,
                });
            }

            let parsed = ids
                .into_iter()
                .map(|id| {
                    Uuid::parse_str(id.trim())
                        .map_err(|_| QueryError::MalformedSubjectId { value: id })
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            Ok(ValidatedSubjects::Persons(parsed.into_iter().collect()))
        }
    }
}

fn window(from: &str, to: &str) -> Result<(NaiveDate, NaiveDate), QueryError> {
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

fn date(field: &'static str, value: &str) -> Result<NaiveDate, QueryError> {
    NaiveDate::parse_from_str(value.trim(), DATE_FORMAT).map_err(|_| QueryError::MalformedDate {
        field,
        value: value.to_owned(),
    })
}

fn validate_split(
    catalog: &MetricCatalog,
    metric_key: &str,
    split: Split,
) -> Result<ValidatedSplit, QueryError> {
    if split.dimensions.is_empty() {
        return Err(QueryError::NoSplitDimensions);
    }
    if split.dimensions.len() > MAX_SPLIT_DIMENSIONS {
        return Err(QueryError::TooManySplitDimensions {
            limit: MAX_SPLIT_DIMENSIONS,
        });
    }

    let mut seen = BTreeSet::new();
    let mut dimensions = Vec::with_capacity(split.dimensions.len());
    for dimension in split.dimensions {
        let dimension = dimension.trim().to_owned();
        if !seen.insert(dimension.clone()) {
            return Err(QueryError::DuplicateSplitDimension { dimension });
        }
        dimensions.push(dimension);
    }

    let limit = split
        .limit
        .map(|limit| validate_limit(catalog, metric_key, limit))
        .transpose()?;
    Ok(ValidatedSplit { dimensions, limit })
}

fn validate_limit(
    catalog: &MetricCatalog,
    metric_key: &str,
    limit: SplitLimit,
) -> Result<ValidatedSplitLimit, QueryError> {
    if !(1..=MAX_SPLIT_TOP).contains(&limit.top) {
        return Err(QueryError::SplitTopOutOfRange {
            limit: MAX_SPLIT_TOP,
        });
    }

    // INVARIANT: the ranking metric is itself defined, or the answer would be
    // ordered by numbers it does not report.
    let rank_by = match limit.rank_by {
        Some(key) => defined_metric(catalog, &key)?,
        None => metric_key.to_owned(),
    };

    Ok(ValidatedSplitLimit {
        top: limit.top,
        rank_by,
        remainder: limit.remainder,
    })
}

/// Which read answers a question, or why none does.
fn shape(
    grain: Grain,
    fold: Fold,
    split: Option<&ValidatedSplit>,
    subjects: &ValidatedSubjects,
) -> Result<QueryShape, QueryError> {
    let shape = match (grain, fold, split) {
        (Grain::Total, Fold::PerSubject, None) => QueryShape::SubjectTotal,
        (Grain::Total, Fold::PerSubject, Some(split)) => {
            if split.limit.is_some() {
                return Err(QueryError::Unanswerable {
                    reason: "a per-subject split over the whole window reports every group; \
                             keeping only the top ones needs a time grain or a combined fold",
                });
            }
            QueryShape::SubjectSplit
        }
        (Grain::Total, Fold::Combined, Some(_)) => QueryShape::CombinedSplit,
        (Grain::Total, Fold::Combined, None) => {
            return Err(QueryError::Unanswerable {
                reason: "a combined value is reported per split group, so folding every subject \
                         together names at least one dimension",
            });
        }
        (Grain::Day | Grain::Week | Grain::Month, Fold::PerSubject, _) => QueryShape::SubjectSeries,
        (Grain::Day | Grain::Week | Grain::Month, Fold::Combined, _) => {
            return Err(QueryError::Unanswerable {
                reason: "a combined value folds the window whole, so it is asked at the total \
                         grain",
            });
        }
    };

    answerable_for(subjects, shape)
}

/// INVARIANT: a dataset records rows per observed person and never for the
/// tenant, so a tenant-wide question must report no subject of its own.
fn answerable_for(
    subjects: &ValidatedSubjects,
    shape: QueryShape,
) -> Result<QueryShape, QueryError> {
    match (subjects, shape) {
        (ValidatedSubjects::Persons(_), _)
        | (ValidatedSubjects::Tenant, QueryShape::CombinedSplit) => Ok(shape),
        (
            ValidatedSubjects::Tenant,
            QueryShape::SubjectTotal | QueryShape::SubjectSplit | QueryShape::SubjectSeries,
        ) => Err(QueryError::Unanswerable {
            reason: "no dataset records a row keyed by the tenant, so a tenant-wide question is \
                     answered with its subjects folded together",
        }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::catalog::product_metric_catalog;
    use super::*;

    const SHIPPED_METRIC: &str = "git.commits";
    const SHIPPED_DIMENSION: &str = "repository";

    fn catalog() -> &'static MetricCatalog {
        product_metric_catalog().expect("the shipped definitions load")
    }

    fn person() -> Uuid {
        Uuid::from_u128(1)
    }

    fn request(body: serde_json::Value) -> Result<ValidatedBatch, QueryError> {
        let parsed: ValuesRequest = serde_json::from_value(body).expect("the wire shape parses");
        validate_request(catalog(), parsed)
    }

    type Refusal = fn(&QueryError) -> bool;

    fn one(query: serde_json::Value) -> Result<ValidatedQuery, QueryError> {
        let body = serde_json::Value::Object(serde_json::Map::from_iter([(
            "queries".to_owned(),
            serde_json::Value::Array(vec![query]),
        )]));
        let mut batch = request(body)?;
        Ok(batch.queries.remove(0))
    }

    fn persons_query(grain: &str, fold: &str, split: serde_json::Value) -> serde_json::Value {
        let mut query = serde_json::json!({
            "metric": SHIPPED_METRIC,
            "subjects": { "type": "persons", "ids": [person().to_string()] },
            "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": grain },
            "fold": fold,
        });
        if !split.is_null() {
            query["split"] = split;
        }
        query
    }

    fn split(limit: Option<serde_json::Value>) -> serde_json::Value {
        let mut split = serde_json::json!({ "dimensions": [SHIPPED_DIMENSION] });
        if let Some(limit) = limit {
            split["limit"] = limit;
        }
        split
    }

    #[test]
    fn a_question_becomes_the_typed_values_it_names() {
        let validated = one(persons_query(
            "total",
            "per_subject",
            serde_json::Value::Null,
        ))
        .expect("a person's window total is answerable");

        assert_eq!(
            validated,
            ValidatedQuery {
                metric_key: SHIPPED_METRIC.to_owned(),
                subjects: ValidatedSubjects::Persons(vec![person()]),
                from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
                to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
                grain: Grain::Total,
                split: None,
                shape: QueryShape::SubjectTotal,
            }
        );
    }

    #[test]
    fn every_answerable_combination_of_grain_fold_and_split_names_one_read() {
        let cases = [
            ("total", "per_subject", None, QueryShape::SubjectTotal),
            (
                "total",
                "per_subject",
                Some(split(None)),
                QueryShape::SubjectSplit,
            ),
            (
                "total",
                "combined",
                Some(split(None)),
                QueryShape::CombinedSplit,
            ),
            ("day", "per_subject", None, QueryShape::SubjectSeries),
            ("week", "per_subject", None, QueryShape::SubjectSeries),
            (
                "month",
                "per_subject",
                Some(split(None)),
                QueryShape::SubjectSeries,
            ),
        ];

        for (grain, fold, split, expected) in cases {
            let query = persons_query(grain, fold, split.unwrap_or(serde_json::Value::Null));
            let validated =
                one(query).unwrap_or_else(|error| panic!("{grain}/{fold} is answerable: {error}"));

            assert_eq!(validated.shape, expected, "{grain}/{fold}");
        }
    }

    #[test]
    fn a_combination_no_dataset_answers_is_refused_rather_than_approximated() {
        let cases = [
            ("total", "combined", None),
            ("day", "combined", None),
            ("week", "combined", Some(split(None))),
        ];

        for (grain, fold, split) in cases {
            let query = persons_query(grain, fold, split.unwrap_or(serde_json::Value::Null));

            assert!(
                matches!(one(query), Err(QueryError::Unanswerable { .. })),
                "should be refused: {grain}/{fold}"
            );
        }
    }

    #[test]
    fn a_tenant_question_is_answered_only_where_the_answer_names_no_subject() {
        let combined = one(serde_json::json!({
            "metric": SHIPPED_METRIC,
            "subjects": { "type": "tenant" },
            "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": "total" },
            "split": { "dimensions": [SHIPPED_DIMENSION] },
            "fold": "combined",
        }))
        .expect("a tenant rollup reports no subject of its own");
        assert_eq!(combined.shape, QueryShape::CombinedSplit);

        for (grain, split) in [
            ("total", serde_json::Value::Null),
            ("total", split(None)),
            ("day", serde_json::Value::Null),
        ] {
            let mut query = serde_json::json!({
                "metric": SHIPPED_METRIC,
                "subjects": { "type": "tenant" },
                "time": { "from": "2026-01-01", "to": "2026-01-31", "grain": grain },
                "fold": "per_subject",
            });
            if !split.is_null() {
                query["split"] = split;
            }

            assert!(
                matches!(one(query), Err(QueryError::Unanswerable { .. })),
                "a per-subject tenant question has no row to key: {grain}"
            );
        }
    }

    #[test]
    fn a_metric_the_definitions_do_not_carry_is_refused_by_key() {
        let mut query = persons_query("total", "per_subject", serde_json::Value::Null);
        query["metric"] = serde_json::json!("git.not_a_shipped_metric");

        assert!(matches!(
            one(query),
            Err(QueryError::UnknownMetric { ref metric })
                if metric == "git.not_a_shipped_metric"
        ));
    }

    #[test]
    fn a_cap_ranked_by_a_metric_the_definitions_do_not_carry_is_refused() {
        let query = persons_query(
            "day",
            "per_subject",
            split(Some(serde_json::json!({
                "top": 5,
                "rank_by": "git.not_a_shipped_metric",
                "remainder": true,
            }))),
        );

        assert!(matches!(one(query), Err(QueryError::UnknownMetric { .. })));
    }

    #[test]
    fn a_cap_that_names_no_ranking_metric_ranks_by_the_metric_it_caps() {
        let query = persons_query(
            "day",
            "per_subject",
            split(Some(serde_json::json!({ "top": 5, "remainder": false }))),
        );

        let validated = one(query).expect("a cap defaults to its own metric");

        assert_eq!(
            validated.split.and_then(|split| split.limit),
            Some(ValidatedSplitLimit {
                top: 5,
                rank_by: SHIPPED_METRIC.to_owned(),
                remainder: false,
            })
        );
    }

    #[test]
    fn a_per_subject_window_split_cannot_be_capped() {
        let query = persons_query(
            "total",
            "per_subject",
            split(Some(serde_json::json!({ "top": 5, "remainder": true }))),
        );

        assert!(matches!(one(query), Err(QueryError::Unanswerable { .. })));
    }

    #[test]
    fn a_request_is_refused_when_it_asks_nothing_or_asks_past_the_batch_cap() {
        assert!(matches!(
            request(serde_json::json!({ "queries": [] })),
            Err(QueryError::NoQueries)
        ));

        let query = persons_query("total", "per_subject", serde_json::Value::Null);
        let queries: Vec<serde_json::Value> = (0..=MAX_QUERIES).map(|_| query.clone()).collect();
        assert!(matches!(
            request(serde_json::json!({ "queries": queries })),
            Err(QueryError::TooManyQueries { limit }) if limit == MAX_QUERIES
        ));
    }

    #[test]
    fn a_window_is_refused_when_it_is_unreadable_reversed_or_longer_than_the_cap() {
        let cases: [(&str, &str, Refusal); 4] = [
            ("2026-13-01", "2026-01-31", |error| {
                matches!(error, QueryError::MalformedDate { .. })
            }),
            ("2026-01-01", "not-a-date", |error| {
                matches!(error, QueryError::MalformedDate { .. })
            }),
            ("2026-02-01", "2026-01-01", |error| {
                matches!(error, QueryError::TimeReversed)
            }),
            ("2026-01-01", "2027-12-31", |error| {
                matches!(error, QueryError::WindowTooLong { .. })
            }),
        ];

        for (from, to, expected) in cases {
            let mut query = persons_query("total", "per_subject", serde_json::Value::Null);
            query["time"]["from"] = serde_json::json!(from);
            query["time"]["to"] = serde_json::json!(to);

            let error = one(query).expect_err("the window is refused");
            assert!(expected(&error), "for {from}..{to}: {error}");
        }
    }

    #[test]
    fn a_subject_list_is_refused_when_it_is_empty_unreadable_or_past_the_cap() {
        let cases: [(serde_json::Value, Refusal); 3] = [
            (serde_json::json!([]), |error| {
                matches!(error, QueryError::NoSubjects)
            }),
            (serde_json::json!(["not-a-person"]), |error| {
                matches!(error, QueryError::MalformedSubjectId { .. })
            }),
            (
                serde_json::json!(
                    (0..=MAX_SUBJECTS)
                        .map(|index| Uuid::from_u128(index as u128).to_string())
                        .collect::<Vec<_>>()
                ),
                |error| matches!(error, QueryError::TooManySubjects { .. }),
            ),
        ];

        for (ids, expected) in cases {
            let mut query = persons_query("total", "per_subject", serde_json::Value::Null);
            query["subjects"]["ids"] = ids;

            let error = one(query).expect_err("the subject list is refused");
            assert!(expected(&error), "{error}");
        }
    }

    #[test]
    fn a_split_is_refused_when_it_names_nothing_repeats_itself_or_caps_out_of_range() {
        let cases: [(serde_json::Value, Refusal); 4] = [
            (serde_json::json!({ "dimensions": [] }), |error| {
                matches!(error, QueryError::NoSplitDimensions)
            }),
            (
                serde_json::json!({ "dimensions": [SHIPPED_DIMENSION, SHIPPED_DIMENSION] }),
                |error| matches!(error, QueryError::DuplicateSplitDimension { .. }),
            ),
            (
                serde_json::json!({
                    "dimensions": [SHIPPED_DIMENSION],
                    "limit": { "top": 0, "remainder": true },
                }),
                |error| matches!(error, QueryError::SplitTopOutOfRange { .. }),
            ),
            (
                serde_json::json!({
                    "dimensions": [SHIPPED_DIMENSION],
                    "limit": { "top": MAX_SPLIT_TOP + 1, "remainder": true },
                }),
                |error| matches!(error, QueryError::SplitTopOutOfRange { .. }),
            ),
        ];

        for (split, expected) in cases {
            let query = persons_query("day", "per_subject", split);

            let error = one(query).expect_err("the split is refused");
            assert!(expected(&error), "{error}");
        }
    }

    #[test]
    fn the_same_person_named_twice_is_read_once() {
        let mut query = persons_query("total", "per_subject", serde_json::Value::Null);
        query["subjects"]["ids"] = serde_json::json!([person().to_string(), person().to_string()]);

        let validated = one(query).expect("a repeated id is one subject");

        assert_eq!(
            validated.subjects,
            ValidatedSubjects::Persons(vec![person()])
        );
    }
}
