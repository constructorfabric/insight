use std::time::Duration;

use serde::Deserialize;
use uuid::Uuid;

const REQUEST_TIMEOUT_SECS: u64 = 10;
const MAX_RESPONSE_BYTES: usize = 1_048_576;
const ADMIN_ROLE_ID: Uuid = Uuid::from_u128(0xa4d1_1000_0000_4000_8000_0000_0000_0001);

#[derive(Debug, Deserialize)]
struct MeResponse {
    roles: Vec<MeRole>,
}

#[derive(Debug, Deserialize)]
struct MeRole {
    role_id: Uuid,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum IdentityError {
    #[error("identity request failed")]
    Request,
    #[error("identity response was invalid")]
    Response,
}

#[derive(Debug)]
pub(crate) struct IdentityClient {
    base_url: String,
    http: reqwest::Client,
}

impl IdentityClient {
    pub(crate) fn new(base_url: &str) -> Result<Self, reqwest::Error> {
        Ok(Self {
            base_url: base_url.trim().trim_end_matches('/').to_owned(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }

    pub(crate) fn is_configured(&self) -> bool {
        !self.base_url.is_empty()
    }

    pub(crate) async fn is_admin(
        &self,
        authorization: Option<&str>,
    ) -> Result<bool, IdentityError> {
        let request = self.http.get(format!("{}/v1/me", self.base_url));
        let request = match authorization {
            Some(value) => request.header(reqwest::header::AUTHORIZATION, value),
            None => request,
        };
        let mut response = request.send().await.map_err(|_| IdentityError::Request)?;
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "identity role lookup failed");
            return Err(IdentityError::Response);
        }

        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| IdentityError::Response)?
        {
            if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
                return Err(IdentityError::Response);
            }
            body.extend_from_slice(&chunk);
        }
        let me: MeResponse = serde_json::from_slice(&body).map_err(|_| IdentityError::Response)?;
        Ok(me.roles.iter().any(|role| role.role_id == ADMIN_ROLE_ID))
    }
}
