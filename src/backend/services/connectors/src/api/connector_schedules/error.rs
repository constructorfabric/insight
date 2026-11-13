use axum::http::header;
use axum::response::{IntoResponse, Response};
use toolkit_canonical_errors::{CanonicalError, Http, Problem};

use crate::api::ADMIN_ONLY;
use crate::api::error::ConnectorScheduleError;
use crate::domain::connector_schedules::Error;

pub(super) fn admin_only() -> CanonicalError {
    ConnectorScheduleError::permission_denied()
        .with_reason(ADMIN_ONLY)
        .create()
}

#[derive(Debug)]
pub(super) struct ScheduleError {
    problem: Box<Problem>,
    revision: Option<u64>,
}

impl From<CanonicalError> for ScheduleError {
    fn from(error: CanonicalError) -> Self {
        Self {
            problem: Box::new(Problem::from(error)),
            revision: None,
        }
    }
}

impl IntoResponse for ScheduleError {
    fn into_response(self) -> Response {
        let response = self.problem.into_response();
        match self.revision {
            Some(revision) => {
                ([(header::ETAG, format!("\"{revision}\""))], response).into_response()
            }
            None => response,
        }
    }
}

pub(super) fn invalid(field: &str, problem: impl Into<String>) -> CanonicalError {
    ConnectorScheduleError::invalid_argument()
        .with_field_violation(field, problem, "INVALID")
        .create()
}

pub(super) fn unsupported_media_type() -> ScheduleError {
    let canonical = ConnectorScheduleError::invalid_argument()
        .with_field_violation("Content-Type", "application/json is required", "INVALID")
        .with_override(Http::status_code(415))
        .create();
    let mut error = ScheduleError::from(canonical);
    "Content-Type application/json is required".clone_into(&mut error.problem.detail);
    error
}

pub(super) fn domain_error(error: Error) -> ScheduleError {
    let error = match error {
        Error::NotFound { .. } => ConnectorScheduleError::not_found(error.to_string())
            .with_resource("connector instance")
            .create(),
        Error::RevisionExhausted => ConnectorScheduleError::failed_precondition()
            .with_precondition_violation("schedule", error.to_string(), "EDIT_UNAVAILABLE")
            .with_override(Http::status_code(409))
            .create(),
        Error::ResetUnavailable => {
            let canonical = ConnectorScheduleError::failed_precondition()
                .with_precondition_violation("schedule", error.to_string(), "EDIT_UNAVAILABLE")
                .with_override(Http::status_code(409))
                .create();
            let mut response = ScheduleError::from(canonical);
            response.problem.detail = error.to_string();
            return response;
        }
        Error::MissingMatch => ConnectorScheduleError::failed_precondition()
            .with_precondition_violation("If-Match", error.to_string(), "REQUIRED")
            .with_override(Http::status_code(428))
            .create(),
        Error::Stale(revision) => {
            let canonical = ConnectorScheduleError::failed_precondition()
                .with_precondition_violation("If-Match", error.to_string(), "STALE")
                .with_override(Http::status_code(412))
                .create();
            let mut problem = Problem::from(canonical);
            problem.context["revision"] = revision.into();
            return ScheduleError {
                problem: Box::new(problem),
                revision: Some(revision),
            };
        }
        Error::InvalidMatch => ConnectorScheduleError::invalid_argument()
            .with_field_violation("If-Match", error.to_string(), "INVALID")
            .create(),
        Error::Invalid(problem) => invalid(&problem.field, problem.problem),
        Error::NotChanged(_) | Error::RestoreFailed(_) => CanonicalError::service_unavailable()
            .with_detail(error.to_string())
            .create(),
        Error::Database(error) => unavailable(&error),
        Error::SyncHistory(error) => unavailable(&error),
        Error::StoredCrons(error) => unavailable(&error),
        Error::Cluster(error) => unavailable(&error),
        Error::Lock(error) => unavailable(&error),
        Error::LockBusy => CanonicalError::service_unavailable()
            .with_detail("The schedule was not changed: the instance is busy or no save slot is free; try again")
            .create(),
        Error::Task(error) => unavailable(&error),
        Error::ClusterIdentity | Error::WorkflowName => unavailable(&error),
    };
    error.into()
}

pub(super) fn unavailable(error: &dyn std::fmt::Display) -> CanonicalError {
    tracing::error!(error = %error, "connector schedule operation failed");
    CanonicalError::service_unavailable()
        .with_detail("failed to read or write connector schedules")
        .create()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_and_missing_reset_target_errors_explain_why_the_schedule_cannot_change() {
        for (error, status, reason) in [
            (Error::LockBusy, 503, "schedule was not changed"),
            (Error::LockBusy, 503, "try again"),
            (
                Error::ResetUnavailable,
                409,
                "Reset requires the deployment schedule annotation",
            ),
        ] {
            let response = domain_error(error);
            assert!(
                response.problem.detail.contains(reason),
                "{}: {reason}",
                response.problem.detail
            );
            assert_eq!(response.into_response().status().as_u16(), status);
        }
    }
}
