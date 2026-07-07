//! Authenticator SDK — the inter-gear contract for the authenticator service.
//!
//! Consumers (today: the future permissions service, which calls session-revoke
//! when a grant changes — see `NGINX_BFF.md` §9.4 / `DD-AUTH-07`) depend on **this
//! crate only**, never on the `authenticator` impl crate. The impl registers a
//! `LocalClient` under [`AuthenticatorClientV1`] in the toolkit `ClientHub`; a
//! remote projection can be swapped in later without touching callers.
//!
//! Step 04 ships the minimal surface: [`AuthenticatorClientV1::revoke_user_sessions`].
//! The list/introspection surface grows with the "finish the auth surface" step.

#![allow(clippy::doc_markdown)]

use async_trait::async_trait;

/// Typed error projection returned by [`AuthenticatorClientV1`].
///
/// The impl maps its internal `CanonicalError`s onto these variants so callers
/// can match ergonomically without depending on the toolkit error crate. The
/// wire form remains RFC 9457 `Problem` on the HTTP boundary; this is only the
/// in-process / SDK projection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthenticatorError {
    /// The caller is not authorized to perform the operation.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// The referenced subject / session was not found.
    #[error("not found: {0}")]
    NotFound(String),
    /// The authenticator's backing store (Redis) is unavailable — fail closed.
    #[error("authenticator unavailable: {0}")]
    Unavailable(String),
    /// Any other failure, carrying a human-readable detail.
    #[error("authenticator error: {0}")]
    Other(String),
}

/// Result of a bulk session-revocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RevokeOutcome {
    /// Number of sessions revoked for the subject (0 when the subject had none).
    pub revoked: u64,
}

/// The authenticator's inter-gear client contract (v1).
///
/// Object-safe (`dyn AuthenticatorClientV1`) so it can live in the `ClientHub`.
#[async_trait]
pub trait AuthenticatorClientV1: Send + Sync + 'static {
    /// Revoke every live session for `person_id` (logout everywhere).
    ///
    /// The instant-propagation lever behind DD-AUTH-07: the permissions service
    /// calls this on a grant change so the user re-logs-in with fresh claims.
    /// Idempotent — revoking a subject with no live sessions returns
    /// `RevokeOutcome { revoked: 0 }`.
    ///
    /// # Errors
    /// Returns [`AuthenticatorError::Unavailable`] when the session store is
    /// unreachable (fail closed), or [`AuthenticatorError::Other`] on an
    /// unexpected backend failure.
    async fn revoke_user_sessions(
        &self,
        person_id: &str,
    ) -> Result<RevokeOutcome, AuthenticatorError>;
}
