use toolkit_canonical_errors::CanonicalError;

use crate::domain::definition::{DefinitionError, DefinitionKind, DefinitionStoreError};
use crate::domain::query::metric_query::MetricRunError;
use crate::domain::query::time_window::WindowError;
use crate::domain::surfaces::CustomError;

#[cfg(test)]
mod tests;

pub(crate) trait ApiErrors {
    fn invalid_field(field: &str, detail: String) -> CanonicalError;

    fn timed_out(detail: &str) -> CanonicalError;

    fn name_taken(name: &str) -> CanonicalError;

    fn missing(resource: &str, detail: String) -> CanonicalError;

    fn oversized(field: &str, detail: &str) -> CanonicalError;

    fn window_error(error: &WindowError) -> CanonicalError {
        let field = match error {
            WindowError::Range(_) | WindowError::Maximum(_) | WindowError::Overflow => "range",
        };

        Self::invalid_field(field, error.to_string())
    }

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

    fn custom_error(error: CustomError) -> CanonicalError {
        match error {
            CustomError::NotFound { kind, id } => {
                Self::missing(&id, format!("{} `{id}` was not found", kind.singular()))
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
            | CustomError::NotEditor
            | CustomError::NotOwner
            | CustomError::Widget(_)
            | CustomError::Range(_)
            | CustomError::Unanswerable(_) => {
                tracing::error!(%error, "reading a metric produced an unrelated failure");
                CanonicalError::internal("metric query execution failed").create()
            }
        }
    }

    fn definition_error(error: DefinitionError) -> CanonicalError {
        Self::invalid_field("name", error.to_string())
    }

    fn malformed_id(kind: DefinitionKind, raw: &str) -> CanonicalError {
        Self::missing(raw, format!("{} `{raw}` was not found", kind.singular()))
    }

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

    fn definition_store_error(error: DefinitionStoreError) -> CanonicalError {
        match error {
            DefinitionStoreError::Database(sea_orm::DbErr::ConnectionAcquire(source)) => {
                tracing::warn!(error = ?source, "definition store connection timed out");
                Self::timed_out("definition store timed out")
            }
            DefinitionStoreError::Database(source) => {
                tracing::error!(error = ?source, "definition store operation failed");
                CanonicalError::internal("definition store operation failed").create()
            }
            DefinitionStoreError::IdTaken(id) => Self::name_taken(&id),
            DefinitionStoreError::Missing(id) => {
                Self::missing(&id, format!("`{id}` does not exist"))
            }
            DefinitionStoreError::Json(source) => {
                tracing::error!(error = ?source, "definition body serialization failed");
                CanonicalError::internal("definition store operation failed").create()
            }
            DefinitionStoreError::TagsFull => {
                Self::invalid_field("tags", DefinitionStoreError::TagsFull.to_string())
            }
            DefinitionStoreError::UnreadableRow(said) => {
                tracing::error!(
                    said,
                    "a definition row holds a value this service never wrote"
                );
                CanonicalError::internal("definition store operation failed").create()
            }
        }
    }
}
