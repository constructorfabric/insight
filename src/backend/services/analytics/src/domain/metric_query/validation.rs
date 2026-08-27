//! The boundary between what a caller wrote and what this service reasons
//! about: parsed dates, parsed person ids, a metric key the definitions carry,
//! and one [`QueryShape`]. INVARIANT: a combination the semantic layer cannot
//! answer never becomes a `ValidatedQuery`.

use std::collections::BTreeSet;

use chrono::{Days, Months, NaiveDate};
use uuid::Uuid;

use crate::domain::compiler::request::DimensionFilter;

use super::catalog::MetricCatalog;
use super::dto::{
    Compare, CompareOffset, DimensionFilter as FilterDto, Fold, Grain, Split, SplitLimit, Subjects,
    ValuesQuery, ValuesRequest,
};
use super::error::QueryError;

const MAX_QUERIES: usize = 50;
const MAX_SUBJECTS: usize = 1000;
const MAX_WINDOW_DAYS: i64 = 400;
const MAX_SPLIT_DIMENSIONS: usize = 10;
const MAX_SPLIT_TOP: u32 = 50;
const MAX_FILTERS: usize = 10;
const MAX_FILTER_VALUES: usize = 100;
const MAX_FILTER_VALUE_BYTES: usize = 512;
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

/// The earlier window a question is compared against, already shifted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComparedWindow {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

#[derive(Debug, PartialEq)]
pub struct ValidatedQuery {
    pub metric_key: String,
    pub subjects: ValidatedSubjects,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub grain: Grain,
    pub filters: Vec<DimensionFilter>,
    pub split: Option<ValidatedSplit>,
    pub compare: Option<ComparedWindow>,
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
    let filters = validate_filters(catalog, &metric_key, query.filters)?;
    let split = query
        .split
        .map(|split| validate_split(catalog, &metric_key, split))
        .transpose()?;
    let compare = query
        .compare
        .map(|compare| compared_window(from, to, compare))
        .transpose()?;
    let shape = shape(query.time.grain, query.fold, split.as_ref(), &subjects)?;

    Ok(ValidatedQuery {
        metric_key,
        subjects,
        from,
        to,
        grain: query.time.grain,
        filters,
        split,
        compare,
        shape,
    })
}

/// INVARIANT: a filter narrows every measure the metric reads in one scan, so
/// it may only name a dimension every one of those measures declares.
fn validate_filters(
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

/// The window the comparison reads. INVARIANT: it spans the same number of
/// days as the current one, so both values fold over comparable spans.
fn compared_window(
    from: NaiveDate,
    to: NaiveDate,
    compare: Compare,
) -> Result<ComparedWindow, QueryError> {
    let shifted = match compare.offset {
        CompareOffset::PreviousPeriod => {
            let length = Days::new((to - from).num_days().unsigned_abs() + 1);
            from.checked_sub_days(length)
                .zip(to.checked_sub_days(length))
        }
        CompareOffset::Month | CompareOffset::Quarter | CompareOffset::Year => {
            let months = Months::new(calendar_months(compare.offset));
            from.checked_sub_months(months)
                .zip(to.checked_sub_months(months))
        }
    };

    let Some((from, to)) = shifted else {
        return Err(QueryError::CompareOutOfRange);
    };
    Ok(ComparedWindow { from, to })
}

fn calendar_months(offset: CompareOffset) -> u32 {
    match offset {
        CompareOffset::PreviousPeriod | CompareOffset::Month => 1,
        CompareOffset::Quarter => 3,
        CompareOffset::Year => 12,
    }
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

    let declared = catalog.dimension_keys(metric_key);
    let mut seen = BTreeSet::new();
    let mut dimensions = Vec::with_capacity(split.dimensions.len());
    for dimension in split.dimensions {
        let dimension = dimension.trim().to_owned();
        if !declared.iter().any(|declared| *declared == dimension) {
            return Err(QueryError::UnknownSplitDimension { dimension });
        }
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

    fn filtered_query(filters: serde_json::Value) -> serde_json::Value {
        let mut query = persons_query("total", "per_subject", serde_json::Value::Null);
        query["filters"] = filters;
        query
    }

    fn compared_query(offset: &str) -> serde_json::Value {
        let mut query = persons_query("total", "per_subject", serde_json::Value::Null);
        query["compare"] = serde_json::json!({ "offset": offset });
        query
    }

    /// A ratio whose inputs declare different dimension sets, so the metric's
    /// capability is narrower than either input's.
    const COMPOSED_METRIC: &str = "git.commits_per_active_day";
    /// Declared by that metric's first input and not by its second.
    const UNSHARED_DIMENSION: &str = "branch_scope";

    #[test]
    fn a_split_naming_a_dimension_the_metric_does_not_declare_is_refused() {
        let query = persons_query(
            "total",
            "per_subject",
            serde_json::json!({ "dimensions": ["not_a_dimension"] }),
        );

        let error = one(query).expect_err("an undeclared dimension names no column");

        assert!(
            matches!(error, QueryError::UnknownSplitDimension { .. }),
            "{error}"
        );
    }

    #[test]
    fn a_composed_metric_accepts_only_the_dimensions_every_input_declares() {
        let mut split = persons_query(
            "total",
            "per_subject",
            serde_json::json!({ "dimensions": [UNSHARED_DIMENSION] }),
        );
        split["metric"] = COMPOSED_METRIC.into();
        let mut filter = persons_query("total", "per_subject", serde_json::Value::Null);
        filter["metric"] = COMPOSED_METRIC.into();
        filter["filters"] =
            serde_json::json!([{ "dimension": UNSHARED_DIMENSION, "values": ["default"] }]);

        assert!(
            matches!(
                one(split).expect_err("one input cannot resolve it"),
                QueryError::UnknownSplitDimension { .. }
            ),
            "a split names a key the metric's capability does not carry"
        );
        assert!(
            matches!(
                one(filter).expect_err("one input cannot resolve it"),
                QueryError::UnknownFilterDimension { .. }
            ),
            "a filter names a key the metric's capability does not carry"
        );

        let mut shared = persons_query(
            "total",
            "per_subject",
            serde_json::json!({ "dimensions": [SHIPPED_DIMENSION] }),
        );
        shared["metric"] = COMPOSED_METRIC.into();
        one(shared).expect("a key both inputs declare stays answerable");
    }

    #[test]
    fn a_filter_becomes_the_dimension_narrowing_the_compiler_reads() {
        let validated = one(filtered_query(serde_json::json!([
            { "dimension": SHIPPED_DIMENSION, "values": ["example/app", "example/api"] },
        ])))
        .expect("a declared dimension narrows the read");

        assert_eq!(
            validated.filters,
            vec![DimensionFilter {
                key: SHIPPED_DIMENSION.to_owned(),
                values: vec!["example/app".to_owned(), "example/api".to_owned()],
            }]
        );
    }

    #[test]
    fn a_question_with_no_filters_narrows_nothing() {
        let validated = one(persons_query(
            "total",
            "per_subject",
            serde_json::Value::Null,
        ))
        .expect("filters are optional");

        assert!(validated.filters.is_empty());
    }

    #[test]
    fn a_filter_that_could_never_narrow_a_read_is_refused() {
        let cases: [(serde_json::Value, Refusal); 6] = [
            (
                serde_json::json!([{ "dimension": "not_a_dimension", "values": ["x"] }]),
                |error| matches!(error, QueryError::UnknownFilterDimension { .. }),
            ),
            (
                serde_json::json!([
                    { "dimension": SHIPPED_DIMENSION, "values": ["x"] },
                    { "dimension": SHIPPED_DIMENSION, "values": ["y"] },
                ]),
                |error| matches!(error, QueryError::DuplicateFilterDimension { .. }),
            ),
            (
                serde_json::json!([{ "dimension": SHIPPED_DIMENSION, "values": [] }]),
                |error| matches!(error, QueryError::NoFilterValues { .. }),
            ),
            (
                serde_json::json!([{
                    "dimension": SHIPPED_DIMENSION,
                    "values": vec!["x"; MAX_FILTER_VALUES + 1],
                }]),
                |error| matches!(error, QueryError::TooManyFilterValues { .. }),
            ),
            (
                serde_json::json!([{
                    "dimension": SHIPPED_DIMENSION,
                    "values": ["x".repeat(MAX_FILTER_VALUE_BYTES + 1)],
                }]),
                |error| matches!(error, QueryError::FilterValueTooLong { .. }),
            ),
            (
                serde_json::Value::Array(
                    (0..=MAX_FILTERS)
                        .map(|index| {
                            serde_json::json!({
                                "dimension": format!("dimension_{index}"),
                                "values": ["x"],
                            })
                        })
                        .collect(),
                ),
                |error| matches!(error, QueryError::TooManyFilters { .. }),
            ),
        ];

        for (filters, refusal) in cases {
            let named = filters.to_string();

            let error = one(filtered_query(filters)).expect_err("should refuse");

            assert!(refusal(&error), "should refuse: {named} — got {error}");
        }
    }

    #[test]
    fn each_offset_shifts_the_window_the_way_it_is_documented() {
        let cases = [
            ("previous_period", "2025-12-01", "2025-12-31"),
            ("month", "2025-12-01", "2025-12-31"),
            ("quarter", "2025-10-01", "2025-10-31"),
            ("year", "2025-01-01", "2025-01-31"),
        ];

        for (offset, from, to) in cases {
            let validated = one(compared_query(offset)).expect("the window shifts");

            assert_eq!(
                validated.compare,
                Some(ComparedWindow {
                    from: NaiveDate::parse_from_str(from, DATE_FORMAT).expect("valid date"),
                    to: NaiveDate::parse_from_str(to, DATE_FORMAT).expect("valid date"),
                }),
                "should shift: {offset}"
            );
        }
    }

    #[test]
    fn a_previous_period_shift_moves_the_window_by_its_own_inclusive_length() {
        let mut query = persons_query("total", "per_subject", serde_json::Value::Null);
        query["time"] = serde_json::json!({
            "from": "2026-03-05",
            "to": "2026-03-06",
            "grain": "total",
        });
        query["compare"] = serde_json::json!({ "offset": "previous_period" });

        let validated = one(query).expect("the window shifts");

        assert_eq!(
            validated.compare,
            Some(ComparedWindow {
                from: NaiveDate::from_ymd_opt(2026, 3, 3).expect("valid date"),
                to: NaiveDate::from_ymd_opt(2026, 3, 4).expect("valid date"),
            }),
            "two inclusive days shift back by two days, leaving no overlap"
        );
    }

    #[test]
    fn a_calendar_shift_clamps_a_day_the_earlier_month_does_not_have() {
        let mut query = persons_query("total", "per_subject", serde_json::Value::Null);
        query["time"] = serde_json::json!({
            "from": "2026-03-31",
            "to": "2026-03-31",
            "grain": "total",
        });
        query["compare"] = serde_json::json!({ "offset": "month" });

        let validated = one(query).expect("the window shifts");

        assert_eq!(
            validated.compare,
            Some(ComparedWindow {
                from: NaiveDate::from_ymd_opt(2026, 2, 28).expect("valid date"),
                to: NaiveDate::from_ymd_opt(2026, 2, 28).expect("valid date"),
            })
        );
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
                filters: Vec::new(),
                split: None,
                compare: None,
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
