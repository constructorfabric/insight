use async_trait::async_trait;
use authenticator_sdk::AuthenticatorClientV1;
use toolkit_canonical_errors::CanonicalError;

use crate::session::SessionManager;

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
    async fn revoke_user_sessions(&self, person_id: &str) -> Result<u64, CanonicalError> {
        self.sessions
            .revoke_user_sessions(person_id)
            .await
            .map_err(|e| {
                CanonicalError::service_unavailable()
                    .with_detail(format!("session store unavailable: {e}"))
                    .create()
            })
    }
}
