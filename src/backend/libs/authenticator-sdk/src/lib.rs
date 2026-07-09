//! Authenticator SDK — the inter-gear contract for the authenticator service.
//!
//! Consumers (today: the future permissions service, which calls session-revoke
//! when a grant changes — see `NGINX_BFF.md` §9.4 / `DD-AUTH-07`) depend on
//! **this crate only**, never on the `authenticator` impl crate. The impl
//! registers a `LocalClient` under [`AuthenticatorClientV1`] in the toolkit
//! `ClientHub`; a remote projection can be swapped in later without touching
//! callers.
//!
//! Errors cross the boundary as `CanonicalError` (RFC 9457 `Problem` on the
//! wire) like every other gear — no bespoke error type (toolkit ADR 0005).
//!
//! Step 04 ships the minimal surface: [`AuthenticatorClientV1::revoke_user_sessions`].
//! The list/introspection surface grows with the "finish the auth surface" step.

#![allow(clippy::doc_markdown)]

use async_trait::async_trait;
use toolkit_canonical_errors::CanonicalError;

/// The authenticator's inter-gear client contract (v1).
///
/// Object-safe (`dyn AuthenticatorClientV1`) so it can live in the `ClientHub`.
#[async_trait]
pub trait AuthenticatorClientV1: Send + Sync + 'static {
    /// Revoke every live session for `person_id` (logout everywhere) and return
    /// the number of sessions revoked.
    ///
    /// The instant-propagation lever behind DD-AUTH-07: the permissions service
    /// calls this on a grant change so the user re-logs-in with fresh claims.
    /// Idempotent — revoking a subject with no live sessions returns `Ok(0)`.
    ///
    /// # Errors
    /// Returns a `CanonicalError` — `ServiceUnavailable` when the session store
    /// is unreachable (fail closed), or `Internal` on an unexpected backend
    /// failure.
    async fn revoke_user_sessions(&self, person_id: &str) -> Result<u64, CanonicalError>;
}
