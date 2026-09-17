//! What the HTTP layer answers with, for the failures more than one endpoint
//! can hit.
//!
//! Each endpoint group answers under its own resource type, so the refusal
//! has to be built from that group's error type — but which failures are the
//! caller's, which are ours, and which are worth logging is one policy, and
//! it lives here.

use toolkit_canonical_errors::CanonicalError;

use crate::domain::definition::{DefinitionError, DefinitionStoreError};

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

    /// A name the store cannot hold.
    fn definition_error(error: DefinitionError) -> CanonicalError {
        Self::invalid_field("name", error.to_string())
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
