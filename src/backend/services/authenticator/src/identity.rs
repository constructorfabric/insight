//! Person resolution, behind a trait.
//!
//! The callback resolves the IdP-authenticated principal to an internal
//! `person_id` + tenant memberships (DESIGN §3.4). Identity's internal
//! service-only endpoints return `insight_source_id` (the person id) but
//! **no tenant memberships**, so:
//!
//! - `person_id` comes from `insight_source_id` — the login-bootstrap resolves
//!   by `(idp.source_type, external_id)` (`GET /internal/persons/by-external-id`);
//!   the admin `__override` view-as resolves by email
//!   (`GET /internal/persons/by-email-override`). Which one applies is decided
//!   by the EXPLICIT [`ResolveTarget`] the caller builds — never inferred from
//!   an empty/absent value, so a login that lacks its external id fails closed
//!   instead of silently falling through to email resolution;
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

/// How to resolve an [`IdpIdentity`] to a person — set explicitly by the
/// caller, never inferred from field emptiness. This is the fix for the
/// login/override confusion: a normal login MUST carry `ExternalId` (built
/// from the validated id_token in `oidc::exchange_code_pkce`, which itself
/// fails closed when the configured claim is absent — see its doc comment);
/// only the admin `__override` synthetic identity carries `Email`.
#[derive(Debug, Clone)]
pub enum ResolveTarget {
    /// Normal login: resolve by the configured `idp.source_type` + the IdP's
    /// source-native external user id (e.g. Entra's `oid`).
    ExternalId(String),
    /// Admin `__override` (view-as, #1941): an operator typed an email —
    /// resolve by email, NOT by external id.
    Email(String),
}

/// The IdP-authenticated principal, distilled from the validated id_token.
#[derive(Debug, Clone)]
pub struct IdpIdentity {
    /// The raw OIDC `sub` claim — logged/audited, but NOT necessarily what
    /// resolves the person (see `resolve_by`).
    pub sub: String,
    pub email: String,
    /// The single tenant asserted by the id_token (`idp.tenant_claim`, or
    /// `idp.default_tenant_id`); empty when the IdP named none — downstream
    /// then fails closed. One and only one tenant per token (EPIC #1583).
    pub tenant_id: String,
    /// Which person-resolution mode applies — see [`ResolveTarget`].
    pub resolve_by: ResolveTarget,
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
/// the login bootstrap (external id → person, before any tenant/caller
/// exists). So this calls one of two **internal, service-only** endpoints —
/// `GET /internal/persons/by-external-id` (login) or
/// `GET /internal/persons/by-email-override` (admin `__override`) — kept as
/// SEPARATE routes (not one endpoint dispatching on a shared parameter) so the
/// two resolution modes can never be confused for one another. Both
/// authenticate with a short-lived **service gateway JWT** the authenticator
/// mints with its own signing key (`sub_type = service`). Tenant-agnostic: the
/// tenant comes from the id_token (see `resolve`), not from Identity.
#[derive(Clone)]
pub struct IdentityPersonResolver {
    base_url: String,
    http: reqwest::Client,
    keystore: Arc<KeyStore>,
    issuer: String,
    audience: String,
    /// `idp.source_type` — scopes the login-bootstrap's by-external-id
    /// resolve. Not used by the `__override` email path.
    source_type: String,
}

/// The internal resolution response — only the field we need.
#[derive(serde::Deserialize)]
struct ResolveProfile {
    insight_source_id: Option<Uuid>,
}

impl IdentityPersonResolver {
    /// `base_url` is the Identity Service root, e.g. `http://identity:8082`.
    /// `keystore` / `issuer` / `audience` are used to mint the service JWT that
    /// authenticates the internal lookup call. `source_type` is `idp.source_type`
    /// — the identity-resolution source the login-bootstrap resolve is scoped to.
    #[must_use]
    pub fn new(
        base_url: &str,
        keystore: Arc<KeyStore>,
        issuer: String,
        audience: String,
        source_type: String,
    ) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
            keystore,
            issuer,
            audience,
            source_type,
        }
    }

    /// Mint a short-lived service gateway JWT (`sub_type = service`) for the
    /// internal Identity lookup, scoped to `tenant_id`.
    fn mint_service_token(&self, tenant_id: &str) -> anyhow::Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before epoch")?
            .as_secs();
        let claims = GatewayClaims {
            sub: Uuid::new_v5(&Uuid::NAMESPACE_URL, b"service:authenticator").to_string(),
            tenant_id: tenant_id.to_owned(),
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

    /// Call an internal resolve endpoint with the given query params.
    async fn resolve_query(
        &self,
        path: &str,
        tenant_id: &str,
        query: &[(&str, &str)],
    ) -> anyhow::Result<Option<Uuid>> {
        if self.base_url.is_empty() {
            return Ok(None);
        }
        let url = format!("{}{path}", self.base_url);
        let token = self.mint_service_token(tenant_id)?;
        let resp = self
            .http
            .get(&url)
            .query(query)
            .bearer_auth(token)
            .send()
            .await
            .context("Identity request")?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        anyhow::ensure!(
            resp.status().is_success(),
            "Identity returned {} for {path}?{query:?}",
            resp.status()
        );
        let profile: ResolveProfile = resp.json().await.context("decode ResolveProfile")?;
        Ok(profile.insight_source_id.filter(|id| !id.is_nil()))
    }

    /// Login-bootstrap lookup: resolve by the configured IdP `source_type` +
    /// the IdP's source-native external user id.
    async fn lookup_person_id_by_external_id(
        &self,
        external_id: &str,
        tenant_id: &str,
    ) -> anyhow::Result<Option<Uuid>> {
        self.resolve_query(
            "/internal/persons/by-external-id",
            tenant_id,
            &[
                ("source_type", &self.source_type),
                ("external_id", external_id),
            ],
        )
        .await
    }

    /// Admin `__override` (view-as) lookup: resolve by email — an operator
    /// types an email, not an IdP external id. A DISTINCT route from the
    /// login-bootstrap lookup above (never dispatched from the same call).
    async fn lookup_person_id_by_email(
        &self,
        email: &str,
        tenant_id: &str,
    ) -> anyhow::Result<Option<Uuid>> {
        self.resolve_query(
            "/internal/persons/by-email-override",
            tenant_id,
            &[("email", email)],
        )
        .await
    }
}

#[async_trait]
impl PersonResolver for IdentityPersonResolver {
    async fn resolve(&self, id: &IdpIdentity) -> anyhow::Result<Option<PersonResolution>> {
        let person_id = match &id.resolve_by {
            ResolveTarget::ExternalId(external_id) => {
                self.lookup_person_id_by_external_id(external_id, &id.tenant_id)
                    .await?
            }
            ResolveTarget::Email(email) => {
                self.lookup_person_id_by_email(email, &id.tenant_id).await?
            }
        };
        let Some(person_id) = person_id else {
            return Ok(None);
        };
        Ok(Some(PersonResolution {
            person_id: person_id.to_string(),
            tenant_id: id.tenant_id.clone(),
        }))
    }
}
