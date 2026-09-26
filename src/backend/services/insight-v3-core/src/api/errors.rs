//! What the HTTP layer answers with, for the failures more than one endpoint
//! can hit.
//!
//! Each endpoint group answers under its own resource type, so the refusal
//! has to be built from that group's error type — but which failures are the
//! caller's, which are ours, and which are worth logging is one policy, and
//! it lives here.

use toolkit_canonical_errors::CanonicalError;

use crate::domain::definition::{DefinitionError, DefinitionStoreError};
use crate::domain::query::metric_query::MetricRunError;
use crate::domain::query::time_window::WindowError;
use crate::domain::surfaces::CustomError;

#[cfg(test)]
mod tests;

/// One endpoint group, as the errors it answers with.
pub(crate) trait ApiErrors {
    /// A value the caller can correct, named by the field carrying it.
    fn invalid_field(field: &str, detail: String) -> CanonicalError;

    /// A wait the server gave up on, which the caller may retry.
    fn timed_out(detail: &str) -> CanonicalError;

    /// A name another definition already holds.
    fn name_taken(name: &str) -> CanonicalError;

    /// A named thing that is not there.
    fn missing(resource: &str, detail: String) -> CanonicalError;

    /// A body or a result past a size limit, named by the field carrying it.
    fn oversized(field: &str, detail: &str) -> CanonicalError;

    /// A window the caller wrote that cannot be read as one.
    fn window_error(error: &WindowError) -> CanonicalError {
        let field = match error {
            WindowError::Range(_) | WindowError::Maximum(_) | WindowError::Overflow => "range",
        };

        Self::invalid_field(field, error.to_string())
    }

    /// A metric read that did not answer: the wait and the size are the
    /// caller's to see, what the warehouse said is ours.
    fn run_error(error: MetricRunError) -> CanonicalError {
        match error {
            MetricRunError::Timeout => Self::timed_out("metric query timed out"),
            MetricRunError::ResultTooLarge => {
                Self::oversized("body", "metric result exceeded the size limit")
            }
            MetricRunError::ClickHouse(source) => {
                tracing::error!(error = ?source, "metric query execution failed");
                CanonicalError::internal("metric query execution failed").create()
            }
            MetricRunError::InvalidResponse(source) => {
                tracing::error!(error = ?source, "metric query result deserialization failed");
                CanonicalError::internal("metric query execution failed").create()
            }
        }
    }

    /// Whatever reading a stored metric can fail with, as the caller sees it.
    fn custom_error(error: CustomError) -> CanonicalError {
        match error {
            CustomError::NotFound { kind, name } => {
                Self::missing(&name, format!("{} `{name}` was not found", kind.singular()))
            }
            CustomError::DatasetNotReady(named) => Self::missing(
                &named,
                format!("no dataset named `{named}` is ready to be read"),
            ),
            CustomError::Body(source) => Self::invalid_field("body", source.to_string()),
            CustomError::Compile(source) => Self::invalid_field("body", source.to_string()),
            CustomError::Run(source) => Self::run_error(source),
            CustomError::Store(source) => Self::definition_store_error(source),
            CustomError::Datasets(source) => Self::dataset_store_error(source),
            CustomError::InUse { .. }
            | CustomError::Widget(_)
            | CustomError::Range(_)
            | CustomError::Unanswerable(_) => {
                tracing::error!(%error, "reading a metric produced an unrelated failure");
                CanonicalError::internal("metric query execution failed").create()
            }
        }
    }

    /// A name the store cannot hold.
    fn definition_error(error: DefinitionError) -> CanonicalError {
        Self::invalid_field("name", error.to_string())
    }

    /// A body axum would not read.
    ///
    /// INVARIANT: an extractor rejects before the handler runs, so a write
    /// takes its body as a `Result` and answers this only once the caller has
    /// proved they may write at all. Otherwise an unauthorized caller learns
    /// the route exists from an unshaped refusal.
    fn unreadable_body(error: &axum::extract::rejection::JsonRejection) -> CanonicalError {
        match error.status() {
            axum::http::StatusCode::PAYLOAD_TOO_LARGE => {
                Self::invalid_field("body", "the request body exceeds the limit".to_owned())
            }
            axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE => Self::invalid_field(
                "body",
                "the content type must be application/json".to_owned(),
            ),
            _ => Self::invalid_field("body", "the request body is not a JSON object".to_owned()),
        }
    }

    /// A dataset store that did not answer.
    ///
    /// Same policy as the definitions below: only the wait is the caller's to
    /// see, and a refusal the caller can act on keeps its own shape.
    fn dataset_store_error(error: crate::domain::datasets::DatasetStoreError) -> CanonicalError {
        use crate::domain::datasets::DatasetStoreError;

        match error {
            DatasetStoreError::Database(sea_orm::DbErr::ConnectionAcquire(source)) => {
                tracing::warn!(error = ?source, "dataset store connection timed out");
                Self::timed_out("dataset store timed out")
            }
            DatasetStoreError::Refused(refusal) => Self::invalid_field("name", refusal.to_string()),
            DatasetStoreError::Database(source) => {
                tracing::error!(error = ?source, "dataset store operation failed");
                CanonicalError::internal("dataset store operation failed").create()
            }
            DatasetStoreError::Json(source) => {
                tracing::error!(error = ?source, "a dataset declaration could not be read");
                CanonicalError::internal("dataset store operation failed").create()
            }
            DatasetStoreError::UnreadableRow(said) => {
                tracing::error!(said, "a dataset row holds a word this service never wrote");
                CanonicalError::internal("dataset store operation failed").create()
            }
        }
    }

    /// A definition store that did not answer.
    ///
    /// Only the wait is the caller's to see: what the database said is ours,
    /// and saying it back would describe our schema to whoever asked.
    fn definition_store_error(error: DefinitionStoreError) -> CanonicalError {
        match error {
            // Waiting for a connection is the store being busy, not broken.
            DefinitionStoreError::Database(sea_orm::DbErr::ConnectionAcquire(source)) => {
                tracing::warn!(error = ?source, "definition store connection timed out");
                Self::timed_out("definition store timed out")
            }
            DefinitionStoreError::Database(source) => {
                tracing::error!(error = ?source, "definition store operation failed");
                CanonicalError::internal("definition store operation failed").create()
            }
            DefinitionStoreError::NameTaken(name) => Self::name_taken(&name),
            DefinitionStoreError::Json(source) => {
                tracing::error!(error = ?source, "definition body serialization failed");
                CanonicalError::internal("definition store operation failed").create()
            }
        }
    }
}
