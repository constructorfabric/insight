//! Person resolution, behind a trait.
//!
//! The callback resolves the IdP-authenticated principal to an internal
//! `person_id` + tenant memberships (DESIGN §3.4). The Identity Service's
//! `GET /v1/persons/{email}` returns `ResolveProfileCommandModel`
//! (`insight_source_id` = the person id) but **no tenant memberships**, so:
//!
//! - `person_id` comes from `insight_source_id` when Identity knows the email;
//! - `tenants` are sourced from the validated id_token claim (fakeidp supplies
//!   it; real-IdP tenant-membership resolution is a follow-up —
//!   constructorfabric/insight#1687);
//! - an unknown person is denied (the callback returns 403). First-admin
//!   bootstrap / RBAC are out of step-04 scope (a separate universe-admin
//!   initiative); local dev seeds the persons table.
//!
//! Sitting behind [`PersonResolver`] lets a richer Identity contract (or the
//! permissions service) swap the impl without touching the callback.

use anyhow::Context as _;
use async_trait::async_trait;
use uuid::Uuid;

/// The IdP-authenticated principal, distilled from the validated id_token.
#[derive(Debug, Clone)]
pub struct IdpIdentity {
    pub sub: String,
    pub email: String,
    /// Tenant memberships as asserted by the id_token (may be empty for real IdPs).
    pub tenants: Vec<String>,
}

/// The resolved internal author of a session.
#[derive(Debug, Clone)]
pub struct PersonResolution {
    pub person_id: String,
    pub tenants: Vec<String>,
}

/// Resolves the IdP principal to an internal person.
#[async_trait]
pub trait PersonResolver: Send + Sync {
    /// Resolve an existing person. `Ok(None)` = unknown person (the callback
    /// then returns 403).
    ///
    /// # Errors
    /// Fails when the Identity Service is unreachable or errors.
    async fn resolve(&self, id: &IdpIdentity) -> anyhow::Result<Option<PersonResolution>>;
}

/// `PersonResolver` backed by the Identity Service.
#[derive(Clone)]
pub struct IdentityPersonResolver {
    base_url: String,
    http: reqwest::Client,
}

/// `GET /v1/persons/{email}` response — only the field we need.
#[derive(serde::Deserialize)]
struct ResolveProfile {
    insight_source_id: Option<Uuid>,
}

impl IdentityPersonResolver {
    /// `base_url` is the Identity Service root, e.g. `http://identity:8082`.
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
        }
    }

    /// Look up the internal person id for an email via Identity.
    async fn lookup_person_id(&self, email: &str) -> anyhow::Result<Option<Uuid>> {
        if self.base_url.is_empty() {
            return Ok(None);
        }
        let encoded = urlencoding_min(email);
        let url = format!("{}/v1/persons/{encoded}", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Identity request")?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        anyhow::ensure!(
            resp.status().is_success(),
            "Identity returned {} for {email}",
            resp.status()
        );
        let profile: ResolveProfile = resp.json().await.context("decode ResolveProfile")?;
        Ok(profile.insight_source_id.filter(|id| !id.is_nil()))
    }
}

#[async_trait]
impl PersonResolver for IdentityPersonResolver {
    async fn resolve(&self, id: &IdpIdentity) -> anyhow::Result<Option<PersonResolution>> {
        let Some(person_id) = self.lookup_person_id(&id.email).await? else {
            return Ok(None);
        };
        Ok(Some(PersonResolution {
            person_id: person_id.to_string(),
            tenants: id.tenants.clone(),
        }))
    }
}

/// Minimal percent-encoding for an email in a path segment (encodes the few
/// characters that actually appear / matter; avoids a dependency).
fn urlencoding_min(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'@' => {
                out.push(b as char);
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}
