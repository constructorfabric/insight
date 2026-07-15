//! Gear configuration — the §4.1 / DESIGN §3.9 table, transcribed 1:1.
//!
//! Loaded via `GearCtx::config_or_default::<AuthenticatorConfig>()`, which
//! deserializes `gears.authenticator.config` and layers
//! `APP__gears__authenticator__config__<field>` env overrides on top (the
//! dash-free gear name is what makes those env keys work).
//!
//! Every field carries the spec default, so an operator gets a holding config
//! by touching nothing but the connection strings and OIDC client secret.

use std::collections::HashMap;

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

/// A service-registry entry: the public identity of one calling service
/// (DESIGN §3.9 / DD-AUTH-05). Public keys are **not** secrets, so the whole
/// registry lives in gitops-reviewable config: onboarding a service is a PR
/// adding its public key; rotation ships key `n+1` alongside `n` (list both),
/// then removes `n` in a later PR.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ServiceRegistryEntry {
    /// Inline SPKI PEM public key(s) the service signs its RFC 7523 assertions
    /// with. Two keys are allowed at once so a rotation overlaps `previous`+
    /// `next`. Prod/gitops uses this (public keys are not secrets, fine to
    /// commit in a chart ConfigMap).
    pub public_keys: Vec<String>,
    /// Public-key PEM file path(s), resolved against `public_key_dir` when
    /// relative. Dev/e2e uses this so no key material is committed — the
    /// keypair is generated at bring-up (like the gateway signing key) and the
    /// public half is mounted here. Merged with `public_keys`.
    pub public_key_paths: Vec<String>,
    /// Roles baked into the issued gateway JWT. `"service"` is always added by
    /// the issuer, so an entry may leave this empty for a plain service token.
    pub roles: Vec<String>,
}

/// Service-token issuance settings (§10 G1, §10 G4, DESIGN §3.9). The token
/// endpoint runs on its own listener (`token_bind_addr`) so it never shares the
/// main port with the browser/gateway surface (§11.8).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServiceTokensConfig {
    /// Bind address of the dedicated second listener (`POST /internal/token`
    /// + `/ready` only). Suggested 8093; must differ from the main `bind_addr`.
    pub token_bind_addr: String,
    /// Expected `aud` of the client assertion — the authenticator token
    /// endpoint URL the calling service is configured with. Must be non-empty
    /// whenever `services` is non-empty (checked in `validate`).
    pub audience: String,
    /// Maximum accepted assertion lifetime (`exp - iat`), in seconds. RFC 7523
    /// assertions are single-use and short-lived; the spec caps this at 60 s.
    pub assertion_max_lifetime_seconds: u64,
    /// TTL of the issued gateway JWT (service tokens), in seconds. Defaults to
    /// the same 300 s as user tokens so downstream sees one lifetime shape.
    pub token_ttl_seconds: u64,
    /// Extra clock-skew grace (seconds) added to the replay-guard TTL so a
    /// still-valid assertion cannot be replayed within its own lifetime.
    pub clock_skew_leeway_seconds: u64,
    /// Directory that relative `public_key_paths` resolve against. Env-
    /// overridable (like `signing_keys_path`) so dev/e2e can point it at a
    /// generated key dir without committing paths.
    pub public_key_dir: String,
    /// The registry: service name -> its public identity. Empty by default;
    /// dev/compose seed a `testclient` entry, prod ships real ones via gitops.
    pub services: HashMap<String, ServiceRegistryEntry>,
}

impl Default for ServiceTokensConfig {
    fn default() -> Self {
        Self {
            token_bind_addr: "0.0.0.0:8093".to_owned(),
            audience: String::new(),
            assertion_max_lifetime_seconds: 60,
            token_ttl_seconds: 300,
            clock_skew_leeway_seconds: 30,
            public_key_dir: String::new(),
            services: HashMap::new(),
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

    /// Service-token issuance (§10 G1): the second listener + registry.
    pub service_tokens: ServiceTokensConfig,
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
            service_tokens: ServiceTokensConfig::default(),
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

        // Service tokens: if any service is registered, the token endpoint must
        // know the `aud` it expects on assertions (its own URL). A registry
        // entry with zero public keys can never authenticate — reject it early.
        let st = &self.service_tokens;
        anyhow::ensure!(
            !st.token_bind_addr.trim().is_empty(),
            "service_tokens.token_bind_addr is required (empty)"
        );
        anyhow::ensure!(
            st.token_bind_addr != self.bind_addr,
            "service_tokens.token_bind_addr ({}) must differ from bind_addr",
            st.token_bind_addr
        );
        if !st.services.is_empty() {
            anyhow::ensure!(
                !st.audience.trim().is_empty(),
                "service_tokens.audience is required when services are registered"
            );
            anyhow::ensure!(
                st.assertion_max_lifetime_seconds > 0,
                "service_tokens.assertion_max_lifetime_seconds must be > 0"
            );
            for (name, entry) in &st.services {
                anyhow::ensure!(
                    !entry.public_keys.is_empty() || !entry.public_key_paths.is_empty(),
                    "service_tokens.services.{name} has no public_keys or public_key_paths"
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// The `gears.authenticator.config` slice of the checked-in dev config —
    /// just enough to deserialize into [`AuthenticatorConfig`].
    #[derive(serde::Deserialize)]
    struct Host {
        gears: Gears,
    }
    #[derive(serde::Deserialize)]
    struct Gears {
        authenticator: GearSection,
    }
    #[derive(serde::Deserialize)]
    struct GearSection {
        config: AuthenticatorConfig,
    }

    /// The dev `config/insight.yaml` must deserialize into the config struct
    /// (guards `deny_unknown_fields` and YAML indentation) and its registry must
    /// build once its `public_key_paths` resolve. No key material is committed,
    /// so the test generates a keypair into a temp `public_key_dir` (exactly
    /// what run-e2e.sh / dev-compose.sh do at bring-up) before building. A
    /// mistake here would otherwise only surface at container boot.
    #[test]
    fn dev_config_service_tokens_deserialize_and_build() {
        use p256::SecretKey;
        use p256::elliptic_curve::Generate as _;
        use p256::pkcs8::{EncodePublicKey as _, LineEnding};

        let raw = include_str!("../config/insight.yaml");
        let host: Host = serde_yaml::from_str(raw).expect("dev config deserializes");
        let mut st = host.gears.authenticator.config.service_tokens;

        assert_eq!(st.token_bind_addr, "0.0.0.0:8093");
        assert!(st.audience.contains("/internal/token"));
        let testclient = st.services.get("testclient").expect("testclient entry");
        assert_eq!(testclient.public_key_paths, vec!["testclient.pub.pem"]);
        assert!(
            testclient.public_keys.is_empty(),
            "no key material should be committed inline in the dev config"
        );

        // Generate the referenced public key into a temp dir, as bring-up does.
        let dir = std::env::temp_dir().join(format!("authn-svc-key-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pub_pem = SecretKey::generate()
            .public_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        std::fs::write(dir.join("testclient.pub.pem"), &pub_pem).unwrap();
        st.public_key_dir = dir.to_string_lossy().into_owned();

        crate::service_token::ServiceRegistry::build(&st)
            .expect("dev registry builds once public_key_paths resolve");
        std::fs::remove_dir_all(&dir).ok();
    }
}
