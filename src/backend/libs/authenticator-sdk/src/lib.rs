#![allow(clippy::doc_markdown)]

pub mod service_client;

pub use service_client::{FetchedToken, ServiceTokenClient};

use async_trait::async_trait;
use toolkit_canonical_errors::CanonicalError;

/// Object-safe (`dyn AuthenticatorClientV1`) so it can live in the `ClientHub`.
#[async_trait]
pub trait AuthenticatorClientV1: Send + Sync + 'static {
    /// Revoke every live session for `person_id` (logout everywhere) and return
    /// the number of sessions revoked.
    /// Idempotent — revoking a subject with no live sessions returns `Ok(0)`.
    ///
    /// # Errors
    /// Returns a `CanonicalError` — `ServiceUnavailable` when the session store
    /// is unreachable (fail closed), or `Internal` on an unexpected backend
    /// failure.
    async fn revoke_user_sessions(&self, person_id: &str) -> Result<u64, CanonicalError>;
}
