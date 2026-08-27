//! Why a values question is not answered.
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
    NoSubjects,
    #[error("at most {limit} people per question")]
    TooManySubjects { limit: usize },
    #[error("`{value}` is not a person id")]
    MalformedSubjectId { value: String },
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
    /// A question nothing in the semantic layer can answer, as asked.
    #[error("{reason}")]
    Unanswerable { reason: &'static str },
    #[error("the question does not compile: {0}")]
    Uncompilable(#[from] CompileError),
    #[error("the people it asks about could not be resolved into identities")]
    SubjectsUnresolved,
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
            Self::UnknownMetric { .. } => "queries.metric",
            Self::MalformedDate { field, .. } => field,
            Self::TimeReversed | Self::WindowTooLong { .. } => "queries.time",
            Self::NoSubjects | Self::TooManySubjects { .. } | Self::MalformedSubjectId { .. } => {
                "queries.subjects.ids"
            }
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
            Self::Uncompilable(_)
            | Self::NoQueries
            | Self::TooManyQueries { .. }
            | Self::Unanswerable { .. }
            | Self::ResultTooLarge { .. }
            | Self::SubjectsUnresolved
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
            QueryError::ResultTooLarge { .. } => MetricError::invalid_argument()
                .with_field_violation(error.field(), error.to_string(), "metric_result_too_large")
                .create(),
            QueryError::SubjectsUnresolved
            | QueryError::SplitUnranked
            | QueryError::ReadFailed
            | QueryError::RowsUndecodable => {
                tracing::error!(%error, "a values question went unanswered");
                Self::internal("metric values query failed").create()
            }
            QueryError::NoQueries
            | QueryError::TooManyQueries { .. }
            | QueryError::MalformedDate { .. }
            | QueryError::TimeReversed
            | QueryError::WindowTooLong { .. }
            | QueryError::NoSubjects
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
            QueryError::NoSubjects,
            QueryError::MalformedSubjectId {
                value: "nobody".to_owned(),
            },
            QueryError::NoSplitDimensions,
            QueryError::SplitTopOutOfRange { limit: 50 },
            QueryError::Unanswerable {
                reason: "nothing keys a row by the tenant",
            },
            QueryError::ResultTooLarge { limit: 5000 },
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
            QueryError::SplitUnranked,
            QueryError::RowsUndecodable,
        ] {
            assert_eq!(status(case), StatusCode::INTERNAL_SERVER_ERROR.as_u16());
        }
    }
}
