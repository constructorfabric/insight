use chrono::NaiveDate;
use toolkit_canonical_errors::CanonicalError;

use crate::api::error::MetricError;

pub fn evidence_unavailable() -> CanonicalError {
    MetricError::failed_precondition()
        .with_precondition_violation(
            "metric evidence",
            "Evidence is not available for this metric.",
            "EVIDENCE_UNAVAILABLE",
        )
        .create()
}

pub(super) fn db_error(error: &sea_orm::DbErr) -> CanonicalError {
    tracing::error!(error = %error, "metric drilldown metadata query failed");
    CanonicalError::internal("failed to load metric evidence metadata").create()
}

pub(super) fn config_error() -> CanonicalError {
    CanonicalError::internal("corrupt metric evidence configuration").create()
}

pub(super) fn parse_date(field: &str, value: &str) -> Result<NaiveDate, CanonicalError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| invalid_error(field, "date must use YYYY-MM-DD"))
}

pub(super) fn invalid<T>(field: &str, description: impl Into<String>) -> Result<T, CanonicalError> {
    Err(invalid_error(field, description))
}

pub(super) fn invalid_error(field: &str, description: impl Into<String>) -> CanonicalError {
    MetricError::invalid_argument()
        .with_field_violation(field, description.into(), "INVALID")
        .create()
}

pub(crate) fn export_limit(description: impl Into<String>) -> CanonicalError {
    MetricError::resource_exhausted("Metric evidence export exceeded resource limits.")
        .with_quota_violation("metric evidence export", description.into())
        .create()
}

pub(crate) fn export_internal() -> CanonicalError {
    CanonicalError::internal("failed to build metric evidence export").create()
}
