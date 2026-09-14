use crate::infra::metrics::ErrorClass;

use super::dto::{MetricResultViewDto, MetricViewErrorCode};

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
    /// Classify a ClickHouse error message (submit or fetch failure).
    pub fn from_query_error(message: &str) -> Self {
        let code = match ErrorClass::classify(message) {
            ErrorClass::RelationMissing => MetricViewErrorCode::SourceRelationMissing,
            ErrorClass::ResourceExhausted => MetricViewErrorCode::ResourceExhausted,
            ErrorClass::Timeout => MetricViewErrorCode::QueryTimeout,
            ErrorClass::ParseFailed => MetricViewErrorCode::ResultParseFailed,
            ErrorClass::QueryFailed => MetricViewErrorCode::QueryFailed,
        };
        Self {
            code,
            detail: message.to_owned(),
        }
    }

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
        MetricViewErrorCode::SourceRelationMissing => {
            "The data behind this metric has not been built yet; it usually appears after the next data refresh."
        }
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

    #[test]
    fn missing_relation_classifies_by_name_or_code() {
        for message in [
            "bad response: Code: 60. DB::Exception: Table insight.x does not exist. (UNKNOWN_TABLE)",
            "Code: 60. something",
        ] {
            let failure = ViewFailure::from_query_error(message);
            assert_eq!(
                failure.code,
                MetricViewErrorCode::SourceRelationMissing,
                "should classify as missing relation: {message}"
            );
        }
    }

    #[test]
    fn resource_limits_classify_as_exhausted() {
        for message in [
            "Code: 241. DB::Exception: Memory limit (for query) exceeded",
            "DB::Exception: Timeout exceeded (TIMEOUT_EXCEEDED)",
            "Code: 202. DB::Exception: Too many simultaneous queries",
        ] {
            let failure = ViewFailure::from_query_error(message);
            assert_eq!(
                failure.code,
                MetricViewErrorCode::ResourceExhausted,
                "should classify as resource limit: {message}"
            );
        }
    }

    #[test]
    fn everything_else_is_a_plain_query_failure() {
        let failure = ViewFailure::from_query_error("Code: 999. DB::Exception: novel");
        assert_eq!(failure.code, MetricViewErrorCode::QueryFailed);
    }

    #[test]
    fn a_longer_code_sharing_a_prefix_does_not_misclassify() {
        for message in [
            "Code: 600. DB::Exception: x",
            "Code: 2410. DB::Exception: y",
        ] {
            let failure = ViewFailure::from_query_error(message);
            assert_eq!(
                failure.code,
                MetricViewErrorCode::QueryFailed,
                "should not match a shorter code's marker: {message}"
            );
        }
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
        let failure = ViewFailure::from_query_error("Code: 241. Memory limit exceeded");

        let MetricResultViewDto::Error { message, .. } = failure.clone().into_view(true) else {
            panic!("expected an error view");
        };
        assert!(message.contains("Memory limit exceeded"));

        let MetricResultViewDto::Error { message, code } = failure.into_view(false) else {
            panic!("expected an error view");
        };
        assert_eq!(code, MetricViewErrorCode::ResourceExhausted);
        assert!(!message.contains("Memory limit"));
        assert!(!message.is_empty());
    }
}
