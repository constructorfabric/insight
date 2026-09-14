//! Identity client: whether the caller holds the admin role.
//!
//! The custom surfaces are admin-only, and roles live in the identity
//! service — the gateway JWT carries a subject and a tenant, not roles. So the
//! caller's own authorization is forwarded to `GET /v1/me` and the answer read
//! off the roles it returns, exactly as the analytics gear does it.

use std::time::Duration;

use serde::Deserialize;
use uuid::Uuid;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The seeded `admin` role id — a stable migration constant of the identity
/// service, mirrored here so a role check needs no extra round trip.
const ADMIN_ROLE_ID: Uuid = Uuid::from_u128(0xa4d1_1000_0000_4000_8000_0000_0000_0001);

#[derive(Debug, Deserialize)]
struct MeResponse {
    roles: Vec<MeRole>,
}

#[derive(Debug, Deserialize)]
struct MeRole {
    role_id: Uuid,
}

#[derive(Debug, Clone)]
pub(crate) struct IdentityClient {
    mode: Mode,
}

#[derive(Debug, Clone)]
enum Mode {
    Live {
        http: reqwest::Client,
        base_url: String,
    },
    /// A fixed answer, so a handler test can be a caller with or without the
    /// role without standing up an identity service.
    #[cfg(test)]
    Fixed(bool),
}

impl IdentityClient {
    pub(crate) fn new(base_url: &str) -> Result<Self, IdentityError> {
        Ok(Self {
            mode: Mode::Live {
                http: reqwest::Client::builder()
                    .timeout(REQUEST_TIMEOUT)
                    .build()?,
                base_url: base_url.trim_end_matches('/').to_owned(),
            },
        })
    }

    #[cfg(test)]
    pub(crate) fn fixed(is_admin: bool) -> Self {
        Self {
            mode: Mode::Fixed(is_admin),
        }
    }

    pub(crate) fn is_configured(&self) -> bool {
        match &self.mode {
            Mode::Live { base_url, .. } => !base_url.is_empty(),
            #[cfg(test)]
            Mode::Fixed(_) => true,
        }
    }

    /// Whether the caller behind `authorization` holds the admin role.
    pub(crate) async fn is_admin(
        &self,
        authorization: Option<&str>,
    ) -> Result<bool, IdentityError> {
        let (http, base_url) = match &self.mode {
            Mode::Live { http, base_url } => (http, base_url),
            #[cfg(test)]
            Mode::Fixed(is_admin) => return Ok(*is_admin),
        };

        let url = format!("{base_url}/v1/me");
        let request = match authorization {
            Some(value) => http.get(&url).header(reqwest::header::AUTHORIZATION, value),
            None => http.get(&url),
        };

        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(IdentityError::Refused(response.status().as_u16()));
        }

        let me: MeResponse = response.json().await?;

        Ok(me.roles.iter().any(|role| role.role_id == ADMIN_ROLE_ID))
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum IdentityError {
    #[error("the identity service could not be reached")]
    Transport(#[from] reqwest::Error),
    #[error("the identity service answered {0}")]
    Refused(u16),
}

impl IdentityError {
    /// Whether the identity service refused the CALLER rather than failing.
    ///
    /// A caller the identity service will not identify has not proved the
    /// role, which is a refusal to report as such — not a fault of ours to
    /// report as a 500.
    pub(crate) fn is_about_the_caller(&self) -> bool {
        matches!(self, Self::Refused(401 | 403))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_admin_role_id_matches_the_identity_services_own_constant() {
        // identity-resolution's roles_repo::ADMIN_ROLE_ID, and the id the
        // front end gates on. A mismatch would deny every caller.
        assert_eq!(
            ADMIN_ROLE_ID.to_string(),
            "a4d11000-0000-4000-8000-000000000001"
        );
    }

    #[test]
    fn a_trailing_slash_does_not_double_up_in_the_path() {
        let client = IdentityClient::new("http://identity-resolution:8082/")
            .unwrap_or_else(|error| panic!("the client builds: {error}"));

        let Mode::Live { base_url, .. } = &client.mode else {
            panic!("a configured client is live");
        };
        assert_eq!(base_url, "http://identity-resolution:8082");
        assert!(client.is_configured());
    }

    #[test]
    fn identity_refusing_the_caller_is_told_apart_from_identity_failing() {
        assert!(IdentityError::Refused(401).is_about_the_caller());
        assert!(IdentityError::Refused(403).is_about_the_caller());
        // A broken identity service is our problem, not the caller's.
        assert!(!IdentityError::Refused(500).is_about_the_caller());
        assert!(!IdentityError::Refused(502).is_about_the_caller());
    }

    #[test]
    fn an_unset_url_is_not_configured() {
        let client =
            IdentityClient::new("").unwrap_or_else(|error| panic!("the client builds: {error}"));

        assert!(!client.is_configured());
    }

    #[test]
    fn the_roles_a_caller_holds_decide() {
        let admin: MeResponse = serde_json::from_value(serde_json::json!({
            "roles": [{ "role_id": "a4d11000-0000-4000-8000-000000000001" }]
        }))
        .unwrap_or_else(|error| panic!("the fixture parses: {error}"));
        let other: MeResponse = serde_json::from_value(serde_json::json!({
            "roles": [{ "role_id": "00000000-0000-4000-8000-000000000009" }]
        }))
        .unwrap_or_else(|error| panic!("the fixture parses: {error}"));

        assert!(admin.roles.iter().any(|role| role.role_id == ADMIN_ROLE_ID));
        assert!(!other.roles.iter().any(|role| role.role_id == ADMIN_ROLE_ID));
    }
}
