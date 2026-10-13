use crate::domain::not_ready::NotReadyReason;
use crate::infra::metrics::ErrorClass;

use super::dto::{MetricResultViewDto, MetricViewErrorCode};

/// Why a view carries no values: the data is not there yet, or the read failed.
///
/// The two are answered differently and must not be collapsed — a missing
/// relation is "no data yet", everything else is a failure the caller is told
/// about.
#[derive(Debug, Clone)]
pub enum ViewAbsence {
    NotReady(NotReadyReason),
    Failed(ViewFailure),
}

impl ViewAbsence {
    /// Read a `ClickHouse` error message (submit or fetch failure).
    pub fn from_query_error(message: &str) -> Self {
        match ErrorClass::classify(message) {
            ErrorClass::RelationMissing => Self::NotReady(NotReadyReason::SourceRelationMissing),
            ErrorClass::ResourceExhausted => {
                Self::failed(MetricViewErrorCode::ResourceExhausted, message)
            }
            ErrorClass::Timeout => Self::failed(MetricViewErrorCode::QueryTimeout, message),
            ErrorClass::ParseFailed => {
                Self::failed(MetricViewErrorCode::ResultParseFailed, message)
            }
            ErrorClass::QueryFailed => Self::failed(MetricViewErrorCode::QueryFailed, message),
        }
    }

    fn failed(code: MetricViewErrorCode, detail: &str) -> Self {
        Self::Failed(ViewFailure {
            code,
            detail: detail.to_owned(),
        })
    }

    pub fn into_view(self, admin: bool) -> MetricResultViewDto {
        match self {
            Self::NotReady(reason) => MetricResultViewDto::NotReady { reason },
            Self::Failed(failure) => failure.into_view(admin),
        }
    }
}

impl From<ViewFailure> for ViewAbsence {
    fn from(failure: ViewFailure) -> Self {
        Self::Failed(failure)
    }
}

/// One view's failed computation: a stable code plus the underlying
/// description. `detail` is shown to admins only; everyone else gets the
/// code's generic message, so ClickHouse internals never reach a regular
/// caller.
#[derive(Debug, Clone)]
pub struct ViewFailure {
    pub code: MetricViewErrorCode,
    pub detail: String,
}

impl ViewFailure {
    pub fn timeout() -> Self {
        Self {
            code: MetricViewErrorCode::QueryTimeout,
            detail: "the query did not answer within the fetch deadline".to_owned(),
        }
    }

    pub fn from_parse_error(message: &str) -> Self {
        Self {
            code: MetricViewErrorCode::ResultParseFailed,
            detail: message.to_owned(),
        }
    }

    /// A failure raised while assembling rows into a view (demux, ranking,
    /// dimension shape) — already logged with full detail at its site, so the
    /// admin message stays at the summary level.
    pub fn from_assembly_error(context: &str) -> Self {
        Self {
            code: MetricViewErrorCode::ResultParseFailed,
            detail: format!("failed to assemble the view from query results: {context}"),
        }
    }

    pub fn into_view(self, admin: bool) -> MetricResultViewDto {
        let message = if admin {
            self.detail
        } else {
            generic_message(self.code).to_owned()
        };
        MetricResultViewDto::Error {
            code: self.code,
            message,
        }
    }
}

fn generic_message(code: MetricViewErrorCode) -> &'static str {
    match code {
        MetricViewErrorCode::ResourceExhausted | MetricViewErrorCode::QueryTimeout => {
            "This metric needed too many resources to compute. Try a shorter period or fewer people."
        }
        MetricViewErrorCode::ResultParseFailed | MetricViewErrorCode::QueryFailed => {
            "This metric could not be computed; the rest of the results are unaffected. An administrator can see the underlying error."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(absence: &ViewAbsence) -> Option<MetricViewErrorCode> {
        match absence {
            ViewAbsence::NotReady(_) => None,
            ViewAbsence::Failed(failure) => Some(failure.code),
        }
    }

    /// An absent table (60) or an absent database (81) is "no data yet", not a
    /// failure — a fresh install has neither.
    #[test]
    fn an_absent_relation_is_not_ready_rather_than_failed() {
        for message in [
            "bad response: Code: 60. DB::Exception: Table insight.x does not exist. (UNKNOWN_TABLE)",
            "Code: 81. DB::Exception: Database insight does not exist. (UNKNOWN_DATABASE)",
            "Code: 60",
        ] {
            assert!(
                matches!(
                    ViewAbsence::from_query_error(message),
                    ViewAbsence::NotReady(NotReadyReason::SourceRelationMissing)
                ),
                "should read as not-ready: {message}"
            );
        }
    }

    /// Code 47 means the relation is there and the deployed code disagrees with
    /// its columns. Answering that with an empty view would hide the one defect
    /// the schema channel exists to catch.
    #[test]
    fn a_column_the_relation_does_not_have_still_fails() {
        let absence = ViewAbsence::from_query_error(
            "Code: 47. DB::Exception: Unknown expression identifier `commits` in scope \
             SELECT commits FROM insight.git (UNKNOWN_IDENTIFIER)",
        );

        assert_eq!(code_of(&absence), Some(MetricViewErrorCode::QueryFailed));
    }

    #[test]
    fn resource_limits_classify_as_exhausted() {
        for message in [
            "Code: 241. DB::Exception: Memory limit (for query) exceeded",
            "DB::Exception: Timeout exceeded (TIMEOUT_EXCEEDED)",
            "Code: 202. DB::Exception: Too many simultaneous queries",
        ] {
            assert_eq!(
                code_of(&ViewAbsence::from_query_error(message)),
                Some(MetricViewErrorCode::ResourceExhausted),
                "should classify as resource limit: {message}"
            );
        }
    }

    #[test]
    fn everything_else_is_a_plain_query_failure() {
        let absence = ViewAbsence::from_query_error("Code: 999. DB::Exception: novel");

        assert_eq!(code_of(&absence), Some(MetricViewErrorCode::QueryFailed));
    }

    /// A failure that merely quotes an absent relation reports its own code, so
    /// it must not be mistaken for one.
    #[test]
    fn a_longer_code_or_a_quoted_absence_does_not_read_as_not_ready() {
        for message in [
            "Code: 600. DB::Exception: x",
            "Code: 2410. DB::Exception: y",
            "Code: 279. DB::Exception: All connection tries failed: Code: 60. (UNKNOWN_TABLE)",
        ] {
            assert_eq!(
                code_of(&ViewAbsence::from_query_error(message)),
                Some(MetricViewErrorCode::QueryFailed),
                "should stay a failure: {message}"
            );
        }
    }

    #[test]
    fn a_not_ready_view_carries_the_marker_and_no_values() {
        let view = ViewAbsence::NotReady(NotReadyReason::SourceRelationMissing).into_view(true);

        let MetricResultViewDto::NotReady { reason } = view else {
            panic!("expected a not-ready view");
        };
        assert_eq!(reason, NotReadyReason::SourceRelationMissing);
    }

    #[test]
    fn each_failure_kind_carries_its_code_and_a_nonempty_generic_message() {
        let failures = [
            ViewFailure::timeout(),
            ViewFailure::from_parse_error("invalid type: null"),
            ViewFailure::from_assembly_error("metric-results:period-batch:m_a"),
        ];
        let expected = [
            MetricViewErrorCode::QueryTimeout,
            MetricViewErrorCode::ResultParseFailed,
            MetricViewErrorCode::ResultParseFailed,
        ];
        for (failure, expected) in failures.into_iter().zip(expected) {
            assert_eq!(failure.code, expected, "wrong code: {failure:?}");
            let MetricResultViewDto::Error { message, .. } = failure.into_view(false) else {
                panic!("expected an error view");
            };
            assert!(!message.is_empty());
        }
    }

    #[test]
    fn admin_sees_the_detail_and_others_the_generic_message() {
        let absence = ViewAbsence::from_query_error("Code: 241. Memory limit exceeded");

        let MetricResultViewDto::Error { message, .. } = absence.clone().into_view(true) else {
            panic!("expected an error view");
        };
        assert!(message.contains("Memory limit exceeded"));

        let MetricResultViewDto::Error { message, code } = absence.into_view(false) else {
            panic!("expected an error view");
        };
        assert_eq!(code, MetricViewErrorCode::ResourceExhausted);
        assert!(!message.contains("Memory limit"));
        assert!(!message.is_empty());
    }
}
