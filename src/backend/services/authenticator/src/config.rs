//! Gear configuration — the §4.1 / DESIGN §3.9 table, transcribed 1:1.
//!
//! Loaded via `GearCtx::config_or_default::<AuthenticatorConfig>()`, which
//! deserializes `gears.authenticator.config` and layers
//! `APP__gears__authenticator__config__<field>` env overrides on top (the
//! dash-free gear name is what makes those env keys work).
//!
//! Every field carries the spec default, so an operator gets a holding config
//! by touching nothing but the connection strings and OIDC client secret.

use serde::Deserialize;

/// Policy for IdPs that issue no refresh token (some withhold `offline_access`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoRefreshTokenPolicy {
    /// Session capped at the IdP access-token lifetime.
    Strict,
    /// Sessions live to the absolute cap; only back-channel logout / manual
    /// revoke kill them early.
    LoginOnly,
}

/// OIDC provider settings and the background-refresh knobs (§4.1 `idp.*`).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IdpConfig {
    /// OIDC issuer URL — discovery root (`{issuer}/.well-known/openid-configuration`).
    pub issuer_url: String,
    /// Confidential-client id registered with the IdP.
    pub client_id: String,
    /// Confidential-client secret (injected per-deployment; never committed).
    pub client_secret: String,
    /// Background refresh of IdP tokens per session (workers land in step 10).
    pub refresh_enabled: bool,
    /// Refresh IdP tokens this long before their expiry.
    pub refresh_safety_margin_seconds: u64,
    /// Max in-flight IdP refresh calls from the leader (politeness, not capacity).
    pub refresh_concurrency: u32,
    /// Behavior when the IdP issues no refresh token.
    pub no_refresh_token_policy: NoRefreshTokenPolicy,
}

impl Default for IdpConfig {
    fn default() -> Self {
        Self {
            issuer_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            refresh_enabled: true,
            refresh_safety_margin_seconds: 60,
            refresh_concurrency: 128,
            no_refresh_token_policy: NoRefreshTokenPolicy::Strict,
        }
    }
}

/// The authenticator gear configuration. Deserialized from
/// `gears.authenticator.config`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthenticatorConfig {
    // ── Session lifecycle (§4.1) ─────────────────────────────────────────
    /// Session token / cookie TTL. Extended only by `POST /auth/refresh`.
    pub session_ttl_seconds: u64,
    /// Hard cap across refreshes; after it, re-login.
    pub session_absolute_lifetime_seconds: u64,
    /// `refresh_at = expires_at - margin (+/- jitter)` handed to the SPA.
    pub session_refresh_safety_margin_seconds: u64,
    /// Full jitter window on `refresh_at`, uniform +/- half.
    pub refresh_jitter_seconds: u64,
    /// TTL applied to the superseded token mapping on rotation (the grace window).
    pub refresh_grace_ms: u64,

    // ── Linked JWT (§4.1 / §3.8) ─────────────────────────────────────────
    /// Linked-JWT validity (`exp - iat`).
    pub jwt_ttl_seconds: u64,
    /// Serve the stored JWT until this age, then reissue ahead of expiry.
    /// Must be `< jwt_ttl_seconds`; the difference is the guaranteed travel margin.
    pub jwt_reissue_after_seconds: u64,
    /// Baked into every JWT until the permissions service replaces the values.
    pub default_roles: Vec<String>,
    /// Upper bound for the gateway-side exchange cache, emitted as
    /// `Cache-Control: max-age` on `/internal/authz` 200s. `0` = per-request.
    pub authz_cache_max_age_seconds: u64,
    /// Gateway origin issuer URL — the JWT `iss` claim.
    pub gateway_issuer: String,
    /// JWT `aud` claim.
    pub jwt_audience: String,

    // ── OIDC handshake ───────────────────────────────────────────────────
    /// The registered redirect URI for the code flow (`{public}/auth/callback`).
    pub redirect_uri: String,
    /// Requested OIDC scopes.
    pub oidc_scopes: Vec<String>,
    /// Where to send the browser after a successful login when the request
    /// named no (or an unsafe) `return_to`. A site-relative path.
    pub default_return_to: String,

    // NOTE: first-admin bootstrap (DD-AUTH-08) and RBAC/ACL are deliberately
    // NOT in step 04 — deferred to a separate universe-admin initiative. Local
    // dev seeds the persons table; an unknown person is denied (403). Every
    // session carries `default_roles` only.

    // ── Cross-cutting ────────────────────────────────────────────────────
    /// CSRF `Origin` allowlist (empty = token-required, fail closed).
    pub csrf_origins: Vec<String>,

    // ── Dependencies ─────────────────────────────────────────────────────
    /// Redis connection URL (`redis://host:port`).
    pub redis_url: String,
    /// Directory holding the ES256 signing keys (`current.pem`, optional
    /// `previous.pem`) — a mounted K8s Secret in production.
    pub signing_keys_path: String,
    /// Identity Service base URL for `sub -> person_id, tenants` resolution.
    pub identity_url: String,

    /// HTTP bind address. Owned by the `api-gateway` host gear; retained for
    /// diagnostics only.
    pub bind_addr: String,

    /// The nested IdP settings.
    pub idp: IdpConfig,
}

impl Default for AuthenticatorConfig {
    fn default() -> Self {
        Self {
            session_ttl_seconds: 600,
            session_absolute_lifetime_seconds: 28800,
            session_refresh_safety_margin_seconds: 90,
            refresh_jitter_seconds: 120,
            refresh_grace_ms: 250,
            jwt_ttl_seconds: 300,
            jwt_reissue_after_seconds: 240,
            default_roles: vec!["user".to_owned()],
            authz_cache_max_age_seconds: 30,
            gateway_issuer: String::new(),
            jwt_audience: "internal-services".to_owned(),
            redirect_uri: String::new(),
            oidc_scopes: vec![
                "openid".to_owned(),
                "email".to_owned(),
                "profile".to_owned(),
                "offline_access".to_owned(),
            ],
            default_return_to: "/".to_owned(),
            csrf_origins: Vec::new(),
            redis_url: String::new(),
            signing_keys_path: String::new(),
            identity_url: String::new(),
            bind_addr: "0.0.0.0:8083".to_owned(),
            idp: IdpConfig::default(),
        }
    }
}

impl AuthenticatorConfig {
    /// Validate cross-field invariants and required fields, so a misconfigured
    /// gear fails fast at boot rather than on the first request.
    ///
    /// # Errors
    /// Returns an error when a lifetime relationship is nonsensical (e.g. the
    /// reissue age is not strictly below the JWT TTL, which would erase the
    /// travel margin) or a required connection/OIDC field is empty.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.jwt_reissue_after_seconds < self.jwt_ttl_seconds,
            "jwt_reissue_after_seconds ({}) must be < jwt_ttl_seconds ({})",
            self.jwt_reissue_after_seconds,
            self.jwt_ttl_seconds
        );
        anyhow::ensure!(
            self.session_ttl_seconds <= self.session_absolute_lifetime_seconds,
            "session_ttl_seconds must be <= session_absolute_lifetime_seconds"
        );

        // Required fields (all injected per-deployment). `idp.client_secret` is
        // intentionally optional — public OIDC clients (e.g. the dev fakeidp)
        // authenticate with PKCE and no secret. `redis_url` is checked in
        // SessionManager::connect.
        for (name, value) in [
            ("gateway_issuer", &self.gateway_issuer),
            ("redirect_uri", &self.redirect_uri),
            ("signing_keys_path", &self.signing_keys_path),
            ("identity_url", &self.identity_url),
            ("idp.issuer_url", &self.idp.issuer_url),
            ("idp.client_id", &self.idp.client_id),
        ] {
            anyhow::ensure!(!value.trim().is_empty(), "{name} is required (empty)");
        }
        Ok(())
    }
}
