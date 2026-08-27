//! Why a question in this family is not answered.
//!
//! INVARIANT: every refusal names the question rather than the machinery, and
//! a read failure carries no server detail — that is logged where it happened.

use toolkit_canonical_errors::CanonicalError;

use crate::api::error::MetricError;
use crate::domain::compiler::error::CompileError;

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("a request asks at least one question")]
    NoQueries,
    #[error("at most {limit} questions per request")]
    TooManyQueries { limit: usize },
    #[error("the definitions carry no metric `{metric}`")]
    UnknownMetric { metric: String },
    #[error("`{field}` is not a date: `{value}`")]
    MalformedDate { field: &'static str, value: String },
    #[error("time.from must be on or before time.to")]
    TimeReversed,
    #[error("a window spans at most {limit} days")]
    WindowTooLong { limit: i64 },
    #[error("a question about people names at least one")]
    NoSubjects { field: &'static str },
    #[error("at most {limit} people per question")]
    TooManySubjects { field: &'static str, limit: usize },
    #[error("`{value}` is not a person id")]
    MalformedSubjectId { field: &'static str, value: String },
    #[error("a split names at least one dimension")]
    NoSplitDimensions,
    #[error("a split names at most {limit} dimensions")]
    TooManySplitDimensions { limit: usize },
    #[error("dimension `{dimension}` is named twice")]
    DuplicateSplitDimension { dimension: String },
    #[error("the metric declares no dimension `{dimension}` to split by")]
    UnknownSplitDimension { dimension: String },
    #[error("a split keeps between 1 and {limit} groups")]
    SplitTopOutOfRange { limit: u32 },
    #[error("a question carries at most {limit} filters")]
    TooManyFilters { limit: usize },
    #[error("the metric declares no dimension `{dimension}` to filter on")]
    UnknownFilterDimension { dimension: String },
    #[error("dimension `{dimension}` is filtered twice")]
    DuplicateFilterDimension { dimension: String },
    #[error("the filter on `{dimension}` names no value, so it can match no row")]
    NoFilterValues { dimension: String },
    #[error("the filter on `{dimension}` names at most {limit} values")]
    TooManyFilterValues { dimension: String, limit: usize },
    #[error("a filter value on `{dimension}` is {length} bytes; at most {limit} are read")]
    FilterValueTooLong {
        dimension: String,
        limit: usize,
        length: usize,
    },
    #[error("the compared window falls outside the range a date can express")]
    CompareOutOfRange,
    #[error(
        "metric `{metric}` declares no peer cohort, so there is no population to compare against"
    )]
    CohortUndeclared { metric: String },
    #[error("a histogram cuts a range into between 1 and {limit} bins")]
    BinsOutOfRange { limit: u32 },
    #[error("a quantile sits strictly between 0 and 1, and `{quantile}` does not")]
    QuantileOutOfRange { quantile: f64 },
    #[error("a question reads at most {limit} quantiles")]
    TooManyQuantiles { limit: usize },
    #[error("a question that names quantiles names at least one")]
    NoQuantiles,
    /// A metric asked for the shape of its own per-row values, which its
    /// computation does not take.
    #[error("{0}")]
    NoDistribution(CompileError),
    /// A question nothing in the semantic layer can answer, as asked.
    #[error("{reason}")]
    Unanswerable { reason: &'static str },
    #[error("the question does not compile: {0}")]
    Uncompilable(#[from] CompileError),
    #[error("the people it asks about could not be resolved into identities")]
    SubjectsUnresolved,
    #[error("the population it compares against could not be resolved")]
    PopulationUnresolved,
    #[error("its population exceeds the {limit} people one comparison may read")]
    PopulationTooLarge { limit: usize },
    #[error("its split could not be ranked")]
    SplitUnranked,
    #[error("the read did not answer")]
    ReadFailed,
    #[error("its rows could not be read back")]
    RowsUndecodable,
    #[error("its answer exceeds the {limit} rows one question may report")]
    ResultTooLarge { limit: usize },
}

impl QueryError {
    fn field(&self) -> &'static str {
        match self {
            Self::UnknownMetric { .. } | Self::NoDistribution(_) => "queries.metric",
            Self::MalformedDate { field, .. }
            | Self::NoSubjects { field }
            | Self::TooManySubjects { field, .. }
            | Self::MalformedSubjectId { field, .. } => field,
            Self::TimeReversed | Self::WindowTooLong { .. } => "queries.time",
            Self::NoSplitDimensions
            | Self::TooManySplitDimensions { .. }
            | Self::DuplicateSplitDimension { .. }
            | Self::UnknownSplitDimension { .. } => "queries.split.dimensions",
            Self::SplitTopOutOfRange { .. } => "queries.split.limit.top",
            Self::TooManyFilters { .. }
            | Self::UnknownFilterDimension { .. }
            | Self::DuplicateFilterDimension { .. }
            | Self::NoFilterValues { .. }
            | Self::TooManyFilterValues { .. }
            | Self::FilterValueTooLong { .. } => "queries.filters",
            Self::CompareOutOfRange => "queries.compare",
            Self::CohortUndeclared { .. } => "queries.population",
            Self::BinsOutOfRange { .. } => "queries.bins",
            Self::QuantileOutOfRange { .. } | Self::TooManyQuantiles { .. } | Self::NoQuantiles => {
                "queries.quantiles"
            }
            Self::Uncompilable(_)
            | Self::NoQueries
            | Self::TooManyQueries { .. }
            | Self::Unanswerable { .. }
            | Self::ResultTooLarge { .. }
            | Self::PopulationTooLarge { .. }
            | Self::SubjectsUnresolved
            | Self::PopulationUnresolved
            | Self::SplitUnranked
            | Self::ReadFailed
            | Self::RowsUndecodable => "queries",
        }
    }
}

impl From<QueryError> for CanonicalError {
    fn from(error: QueryError) -> Self {
        match &error {
            QueryError::UnknownMetric { metric } => MetricError::not_found("metric not found")
                .with_resource(metric.clone())
                .create(),
            QueryError::ResultTooLarge { .. } | QueryError::PopulationTooLarge { .. } => {
                MetricError::invalid_argument()
                    .with_field_violation(
                        error.field(),
                        error.to_string(),
                        "metric_result_too_large",
                    )
                    .create()
            }
            QueryError::SubjectsUnresolved
            | QueryError::PopulationUnresolved
            | QueryError::SplitUnranked
            | QueryError::ReadFailed
            | QueryError::RowsUndecodable => {
                tracing::error!(%error, "a metric question went unanswered");
                Self::internal("metric query failed").create()
            }
            QueryError::NoQueries
            | QueryError::TooManyQueries { .. }
            | QueryError::MalformedDate { .. }
            | QueryError::TimeReversed
            | QueryError::WindowTooLong { .. }
            | QueryError::NoSubjects { .. }
            | QueryError::TooManySubjects { .. }
            | QueryError::MalformedSubjectId { .. }
            | QueryError::NoSplitDimensions
            | QueryError::TooManySplitDimensions { .. }
            | QueryError::DuplicateSplitDimension { .. }
            | QueryError::UnknownSplitDimension { .. }
            | QueryError::SplitTopOutOfRange { .. }
            | QueryError::TooManyFilters { .. }
            | QueryError::UnknownFilterDimension { .. }
            | QueryError::DuplicateFilterDimension { .. }
            | QueryError::NoFilterValues { .. }
            | QueryError::TooManyFilterValues { .. }
            | QueryError::FilterValueTooLong { .. }
            | QueryError::CompareOutOfRange
            | QueryError::CohortUndeclared { .. }
            | QueryError::BinsOutOfRange { .. }
            | QueryError::QuantileOutOfRange { .. }
            | QueryError::TooManyQuantiles { .. }
            | QueryError::NoQuantiles
            | QueryError::NoDistribution(_)
            | QueryError::Unanswerable { .. }
            | QueryError::Uncompilable(_) => MetricError::invalid_argument()
                .with_field_violation(error.field(), error.to_string(), "INVALID")
                .create(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use axum::http::StatusCode;

    use super::*;

    fn status(error: QueryError) -> u16 {
        CanonicalError::from(error).status_code()
    }

    #[test]
    fn a_metric_the_definitions_do_not_carry_is_not_found() {
        assert_eq!(
            status(QueryError::UnknownMetric {
                metric: "git.not_a_metric".to_owned(),
            }),
            StatusCode::NOT_FOUND.as_u16()
        );
    }

    #[test]
    fn a_question_the_caller_could_restate_is_refused_as_a_bad_request() {
        let cases = [
            QueryError::NoQueries,
            QueryError::TooManyQueries { limit: 50 },
            QueryError::TimeReversed,
            QueryError::WindowTooLong { limit: 400 },
            QueryError::NoSubjects {
                field: "queries.subjects.ids",
            },
            QueryError::MalformedSubjectId {
                field: "queries.targets",
                value: "nobody".to_owned(),
            },
            QueryError::NoSplitDimensions,
            QueryError::SplitTopOutOfRange { limit: 50 },
            QueryError::CohortUndeclared {
                metric: "git.commits".to_owned(),
            },
            QueryError::BinsOutOfRange { limit: 100 },
            QueryError::QuantileOutOfRange { quantile: 1.5 },
            QueryError::TooManyQuantiles { limit: 10 },
            QueryError::NoQuantiles,
            QueryError::NoDistribution(CompileError::UnsupportedView {
                metric: "git.commits".to_owned(),
                view: "distributions",
                reason: "it needs a percentile or stddev computation",
            }),
            QueryError::Unanswerable {
                reason: "nothing keys a row by the tenant",
            },
            QueryError::ResultTooLarge { limit: 5000 },
            QueryError::PopulationTooLarge { limit: 5000 },
        ];

        for case in cases {
            let named = case.to_string();
            assert_eq!(
                status(case),
                StatusCode::BAD_REQUEST.as_u16(),
                "should be a bad request: {named}"
            );
        }
    }

    #[test]
    fn a_read_that_did_not_answer_tells_the_caller_nothing_about_the_server() {
        for case in [
            QueryError::ReadFailed,
            QueryError::SubjectsUnresolved,
            QueryError::PopulationUnresolved,
            QueryError::SplitUnranked,
            QueryError::RowsUndecodable,
        ] {
            assert_eq!(status(case), StatusCode::INTERNAL_SERVER_ERROR.as_u16());
        }
    }

    #[test]
    fn a_refusal_names_the_field_the_question_wrote_it_in() {
        let cases = [
            (
                QueryError::NoSubjects {
                    field: "queries.subjects.ids",
                },
                "queries.subjects.ids",
            ),
            (
                QueryError::NoSubjects {
                    field: "queries.targets",
                },
                "queries.targets",
            ),
            (
                QueryError::CohortUndeclared {
                    metric: "git.commits".to_owned(),
                },
                "queries.population",
            ),
            (QueryError::BinsOutOfRange { limit: 100 }, "queries.bins"),
            (QueryError::NoQuantiles, "queries.quantiles"),
        ];

        for (error, field) in cases {
            let named = error.to_string();
            assert_eq!(error.field(), field, "{named}");
        }
    }
}
