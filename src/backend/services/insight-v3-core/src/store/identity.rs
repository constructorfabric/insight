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
/// Who a harness's fixed answer says is asking.
#[cfg(test)]
const FIXED_CALLER: Uuid = Uuid::nil();

#[derive(Debug, Deserialize)]
struct MeResponse {
    person_id: Uuid,
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

    /// The caller behind `authorization` when they hold the admin role, and
    /// nothing when they do not.
    ///
    /// Who asked is answered here rather than left to the caller's word: a
    /// removal takes a dataset's records with it, so "who removed this" has
    /// to have an answer.
    pub(crate) async fn admin_caller(
        &self,
        authorization: Option<&str>,
    ) -> Result<Option<Uuid>, IdentityError> {
        let (http, base_url) = match &self.mode {
            Mode::Live { http, base_url } => (http, base_url),
            #[cfg(test)]
            Mode::Fixed(is_admin) => {
                return Ok(is_admin.then_some(FIXED_CALLER));
            }
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
        let is_admin = me.roles.iter().any(|role| role.role_id == ADMIN_ROLE_ID);

        Ok(is_admin.then_some(me.person_id))
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
mod tests;
