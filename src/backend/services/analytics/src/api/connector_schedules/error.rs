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
        Error::Database(error) => unavailable(&error),
        Error::SyncHistory(error) => unavailable(&error),
        Error::StoredCrons(error) => unavailable(&error),
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
    fn schedule_failures_keep_their_http_status_and_safe_wire_detail()
    -> Result<(), Box<dyn std::error::Error>> {
        for (error, status) in [
            (
                Error::NotFound {
                    connector: "example-tracker".to_owned(),
                    source_id: "main".to_owned(),
                },
                404,
            ),
            (Error::MissingMatch, 428),
            (Error::Stale(4), 412),
            (Error::InvalidMatch, 400),
            (Error::RevisionExhausted, 409),
            (
                Error::StoredCrons(
                    serde_json::from_str::<Vec<String>>("{}")
                        .err()
                        .ok_or("expected invalid stored crons")?,
                ),
                503,
            ),
            (
                Error::Invalid(
                    crate::domain::connector_schedules::validation::Violation::new(
                        "crons[0]",
                        "invalid cron",
                    ),
                ),
                400,
            ),
            (
                Error::Database(sea_orm::DbErr::Custom("synthetic private error".to_owned())),
                503,
            ),
            (
                Error::SyncHistory(clickhouse::error::Error::BadResponse(
                    "synthetic private error".to_owned(),
                )),
                503,
            ),
        ] {
            let problem = serde_json::to_value(domain_error(error).problem)?;
            assert_eq!(problem["status"], status, "{problem}");
            assert!(!problem.to_string().contains("synthetic private error"));
        }
        Ok(())
    }
    #[tokio::test]
    async fn stale_errors_return_the_current_revision_in_context_and_etag()
    -> Result<(), Box<dyn std::error::Error>> {
        for revision in [0, 4, u64::MAX] {
            let response = domain_error(Error::Stale(revision)).into_response();
            assert_eq!(response.status(), 412);
            assert_eq!(response.headers()["etag"], format!("\"{revision}\""));
            let problem = crate::api::connector_schedules::support::body(response).await?;
            assert_eq!(problem["context"]["revision"], revision);
            assert_eq!(problem["context"]["violations"][0]["subject"], "If-Match");
        }
        Ok(())
    }

    #[test]
    fn missing_instance_errors_explain_how_to_find_configured_ids() {
        let problem = domain_error(Error::NotFound {
            connector: "example-tracker".to_owned(),
            source_id: "main".to_owned(),
        })
        .problem;
        for text in [
            "example-tracker/main",
            "configured on this install",
            "Connector health",
            "connector and source id",
        ] {
            assert!(problem.detail.contains(text), "missing {text}: {problem:?}");
        }
    }
}
