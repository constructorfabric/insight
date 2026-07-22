//! Person resolution, behind a trait.
//!
//! The callback resolves the IdP-authenticated principal to an internal
//! `person_id` + tenant memberships (DESIGN §3.4). Identity's internal
//! `GET /internal/persons/by-email/{email}` (service-only) returns
//! `insight_source_id` (the person id) but **no tenant memberships**, so:
//!
//! - `person_id` comes from `insight_source_id` when Identity knows the email;
//! - the single `tenant_id` is sourced from the validated id_token claim
//!   (fakeidp supplies
//!   it; real-IdP tenant-membership resolution is a follow-up —
//!   constructorfabric/insight#1687);
//! - an unknown person is denied (the callback returns 403). First-admin
//!   bootstrap / RBAC are out of step-04 scope (a separate universe-admin
//!   initiative); local dev seeds the persons table.
//!
//! Sitting behind [`PersonResolver`] lets a richer Identity contract (or the
//! permissions service) swap the impl without touching the callback.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use async_trait::async_trait;
use uuid::Uuid;

use crate::jwt::{GatewayClaims, KeyStore};

/// The IdP-authenticated principal, distilled from the validated id_token.
#[derive(Debug, Clone)]
pub struct IdpIdentity {
    pub sub: String,
    pub email: String,
    /// The single tenant asserted by the id_token (`idp.tenant_claim`, or
    /// `idp.default_tenant_id`); empty when the IdP named none — downstream
    /// then fails closed. One and only one tenant per token (EPIC #1583).
    pub tenant_id: String,
}

/// The resolved internal author of a session.
#[derive(Debug, Clone)]
pub struct PersonResolution {
    pub person_id: String,
    pub tenant_id: String,
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
///
/// Identity is fail-closed (NGINX_BFF R1), and its user-facing
/// `/v1/persons/{email}` is tenant + caller + visibility gated — unusable for
/// the login bootstrap (email → person, before any tenant/caller exists). So
/// this calls the **internal, service-only** endpoint
/// `GET /internal/persons/by-email/{email}`, authenticating with a short-lived
/// **service gateway JWT** the authenticator mints with its own signing key
/// (`sub_type = service`). Tenant-agnostic: the tenant comes from the id_token
/// (see `resolve`), not from Identity.
#[derive(Clone)]
pub struct IdentityPersonResolver {
    base_url: String,
    http: reqwest::Client,
    keystore: Arc<KeyStore>,
    issuer: String,
    audience: String,
}

/// The internal resolution response — only the field we need.
#[derive(serde::Deserialize)]
struct ResolveProfile {
    insight_source_id: Option<Uuid>,
}

impl IdentityPersonResolver {
    /// `base_url` is the Identity Service root, e.g. `http://identity:8082`.
    /// `keystore` / `issuer` / `audience` are used to mint the service JWT that
    /// authenticates the internal lookup call.
    #[must_use]
    pub fn new(base_url: &str, keystore: Arc<KeyStore>, issuer: String, audience: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
            keystore,
            issuer,
            audience,
        }
    }

    /// Mint a short-lived service gateway JWT for the internal Identity call.
    /// `sub` is the authenticator's stable service UUID; `sub_type = service`
    /// is what the internal endpoint gates on. Tenant-agnostic, so `tenant_id`
    /// is empty (the endpoint does not read it).
    fn mint_service_token(&self) -> anyhow::Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before epoch")?
            .as_secs();
        let claims = GatewayClaims {
            sub: Uuid::new_v5(&Uuid::NAMESPACE_URL, b"service:authenticator").to_string(),
            tenant_id: String::new(),
            roles: vec!["service".to_owned()],
            sub_type: "service".to_owned(),
            sid: "service:authenticator".to_owned(),
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            iat: now,
            exp: now + 60,
            jti: Uuid::now_v7().to_string(),
        };
        self.keystore.sign(&claims)
    }

    /// Look up the internal person id for an email via Identity's internal
    /// service-only endpoint.
    async fn lookup_person_id(&self, email: &str) -> anyhow::Result<Option<Uuid>> {
        if self.base_url.is_empty() {
            return Ok(None);
        }
        let encoded = urlencoding_min(email);
        let url = format!("{}/internal/persons/by-email/{encoded}", self.base_url);
        let token = self.mint_service_token()?;
        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
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
            tenant_id: id.tenant_id.clone(),
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
