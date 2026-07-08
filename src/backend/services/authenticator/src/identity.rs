//! Person resolution and the first-admin bootstrap, behind a trait.
//!
//! The callback resolves the IdP-authenticated principal to an internal
//! `person_id` + tenant memberships (DESIGN §3.4). The Identity Service's
//! `GET /v1/persons/{email}` returns `ResolveProfileCommandModel`
//! (`insight_source_id` = the person id) but **no tenant memberships and no
//! count/create API**. Per the step-04 brief we therefore:
//!
//! - take `person_id` from `insight_source_id` when Identity knows the email;
//! - source `tenants` from the validated id_token claim (fakeidp supplies it;
//!   real-IdP tenant resolution is a follow-up — see [`BOOTSTRAP_FOLLOWUP`]);
//! - gate first-admin bootstrap on the **config flag only** (no emptiness API
//!   exists), loudly audited, with the INSTALLER as the real production path.
//!
//! All of this sits behind [`PersonResolver`] so a richer Identity contract (or
//! the permissions service) swaps the impl without touching the callback.

use anyhow::Context as _;
use async_trait::async_trait;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

/// Tracking note for the follow-up issue: real emptiness guard + Identity
/// create/count API + first-class tenant-membership resolution.
pub const BOOTSTRAP_FOLLOWUP: &str =
    "authenticator bootstrap uses config-flag gating (no Identity count/create API) and \
     id_token-sourced tenants; replace with the INSTALLER + Identity membership API \
     (constructorfabric/insight#1687)";

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
    /// True only on the audited first-admin bootstrap path.
    pub is_universe_admin: bool,
}

/// Resolves the IdP principal to an internal person, and performs the audited
/// first-admin bootstrap when permitted.
#[async_trait]
pub trait PersonResolver: Send + Sync {
    /// Resolve an existing person. `Ok(None)` = unknown person (-> 403 unless
    /// bootstrap applies).
    ///
    /// # Errors
    /// Fails when the Identity Service is unreachable or errors.
    async fn resolve(&self, id: &IdpIdentity) -> anyhow::Result<Option<PersonResolution>>;

    /// Create the first admin for a fresh install (config-flag gated, audited).
    ///
    /// # Errors
    /// Fails when the backing create path (best-effort) errors unexpectedly.
    async fn bootstrap_first_admin(&self, id: &IdpIdentity) -> anyhow::Result<PersonResolution>;
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
        let resp = self.http.get(&url).send().await.context("Identity request")?;
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
            is_universe_admin: false,
        }))
    }

    async fn bootstrap_first_admin(&self, id: &IdpIdentity) -> anyhow::Result<PersonResolution> {
        // No Identity count/create API exists (see BOOTSTRAP_FOLLOWUP), so the
        // person id is derived deterministically from the IdP subject: stable
        // across the admin's re-logins without persisting anything here. The
        // INSTALLER is the production path that populates the persons table.
        Ok(PersonResolution {
            person_id: deterministic_person_id(&id.sub).to_string(),
            tenants: id.tenants.clone(),
            is_universe_admin: true,
        })
    }
}

/// Derive a stable internal person id from an IdP subject (SHA-256 -> UUID).
fn deterministic_person_id(sub: &str) -> Uuid {
    let digest = Sha256::digest(format!("authenticator:bootstrap:{sub}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
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
