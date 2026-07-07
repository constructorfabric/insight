//! In-process implementation of the SDK contract, registered in the `ClientHub`.
//!
//! The permissions service (future) resolves `dyn AuthenticatorClientV1` from
//! the hub to revoke a user's sessions on a grant change. For step 04 it is the
//! wiring that proves the contract; no remote consumer exists yet.

use async_trait::async_trait;
use authenticator_sdk::{AuthenticatorClientV1, AuthenticatorError, RevokeOutcome};

use crate::session::SessionManager;

/// Local client backed directly by the [`SessionManager`].
pub struct LocalClient {
    sessions: SessionManager,
}

impl LocalClient {
    #[must_use]
    pub fn new(sessions: SessionManager) -> Self {
        Self { sessions }
    }
}

#[async_trait]
impl AuthenticatorClientV1 for LocalClient {
    async fn revoke_user_sessions(
        &self,
        person_id: &str,
    ) -> Result<RevokeOutcome, AuthenticatorError> {
        self.sessions
            .revoke_user_sessions(person_id)
            .await
            .map(|revoked| RevokeOutcome { revoked })
            .map_err(|e| AuthenticatorError::Unavailable(e.to_string()))
    }
}
