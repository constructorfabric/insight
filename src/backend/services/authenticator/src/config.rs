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

/// How a login resolves to a person (§4.1 `idp.resolve_by`).
///
/// A declared mode, not a fallback chain: the install states which question
/// the login bootstrap asks, and a token that cannot answer it is refused
/// rather than quietly answered a different way. Trying one and then the other
/// is what made a login resolvable by an address it was never meant to be
/// resolvable by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveBy {
    /// The IdP's own stable external user id under `idp.source_type`. The
    /// default, and the right answer whenever a connector observes the
    /// provider's accounts: the id is immutable and belongs to the directory
    /// the person authenticated against.
    ExternalId,
    /// The token's standard `email` claim, matched against the addresses the
    /// install's ROSTER states (identity-resolution's `roster_source_type`).
    ///
    /// For installs whose IdP has no directory connector of its own — nothing
    /// ever seeds a `value_type='id'` row for the provider, so `ExternalId`
    /// matches nobody and every sign-in is refused. The address is weaker
    /// evidence than a directory id (it can be reassigned when someone
    /// leaves), which is why it is confined to the one source already trusted
    /// to say who exists rather than to any source that ever stated one.
    Email,
}

/// One host-keyed issuer entry: the issuer and its client registration —
/// the only per-realm settings; everything else in [`IdpConfig`] is global.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HostIdpConfig {
    /// OIDC issuer URL of this host's realm (discovery root; byte-exact match
    /// against the realm's own discovery document, like the flat `issuer_url`).
    pub issuer_url: String,
    /// Confidential-client id registered with this realm.
    pub client_id: String,
    /// Confidential-client secret; empty = public client + PKCE.
    pub client_secret: String,
    /// The redirect URI registered with this realm's client; empty = the
    /// global `redirect_uri`.
    pub redirect_uri: String,
    /// Fallback tenant for this realm when the id_token carries no tenant
    /// claim; empty = the global `idp.default_tenant_id`.
    pub default_tenant_id: String,
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
    /// id_token claim naming the user's single tenant. A plain string (an
    /// array is tolerated: first entry wins). Keycloak emits `tenant_id`;
    /// Entra emits `tid`.
    pub tenant_claim: String,
    /// The `insight_source_type` this IdP is known to identity-resolution as
    /// (e.g. `ms-entra`) — the connector whose `identity_inputs` seed the
    /// matching `persons` rows. Login resolution calls
    /// `GET /internal/persons/by-external-id?source_type=<this>&external_id=<external_id>`;
    /// required (the login-bootstrap has no other way to know which source
    /// the caller authenticated against).
    pub source_type: String,
    /// id_token claim carrying the IdP's stable external user id for
    /// `source_type` — the join key `identity_inputs` seeded it under (e.g.
    /// Entra's `oid`; the generic OIDC `sub` is NOT the same thing for
    /// directory-backed IdPs, see the `ms-entra` connector schema). Defaults
    /// to `sub` (fine for IdPs where `sub` IS the stable directory id, e.g.
    /// Keycloak). Unused when `resolve_by` is `email`.
    pub external_id_claim: String,
    /// Which question the login bootstrap asks to find the person — see
    /// [`ResolveBy`]. Defaults to `external_id`, so an install that says
    /// nothing keeps the directory-id behaviour it had before this existed.
    pub resolve_by: ResolveBy,
    // INVARIANT: off by default — it widens who may ENTER, which is a
    // deployment's policy to set. Identity refuses to mint for a principal no
    // connector has observed, so it never widens who exists.
    pub provision_on_login: bool,
    /// Fallback tenant when the id_token carries no tenant claim at all (e.g.
    /// Okta). Empty = no fallback: the gateway JWT gets an empty `tenant_id`
    /// and downstream services fail closed. Interim until the Identity
    /// membership API (#1687) / Keycloak broker (#1782).
    pub default_tenant_id: String,
    /// PEM bundle of extra CA certificate(s) to trust for the IdP's TLS
    /// connection, on top of the platform's default trust store. Needed
    /// when `issuer_url` sits behind an internal/corporate CA the
    /// container's OS-level trust store doesn't chain to. Empty = trust
    /// only the default store. Mount the PEM file into the pod and point
    /// this at its path.
    pub extra_ca_cert_path: String,
    /// Host-keyed issuer map (ADR-0003). Empty (default) = single-issuer
    /// mode: the flat fields above match EVERY host. Non-empty = exact match
    /// on the normalized `Host` only, unlisted hosts fail closed, and the
    /// flat client fields take no part in selection. Keys are bare
    /// hostnames. Also accepts a JSON string, so one `APP__…__idp__hosts`
    /// env var carries the whole map (env layers can't nest maps).
    #[serde(deserialize_with = "de_hosts")]
    pub hosts: HashMap<String, HostIdpConfig>,
    /// Background refresh of IdP tokens per session (workers land in step 10).
    pub refresh_enabled: bool,
    /// Refresh IdP tokens this long before their expiry.
    pub refresh_safety_margin_seconds: u64,
    /// Max in-flight IdP refresh calls from the leader (politeness, not capacity).
    pub refresh_concurrency: u32,
    /// Behavior when the IdP issues no refresh token.
    pub no_refresh_token_policy: NoRefreshTokenPolicy,
    /// Refresher pass interval (leader polls the due schedule this often).
    pub refresher_tick_seconds: u64,
    /// Jitter (± this window) applied to due-times when WRITTEN to the
    /// schedule, so sessions do not herd after a deploy or Redis restore (G5).
    pub refresh_due_jitter_seconds: u64,
}

impl Default for IdpConfig {
    fn default() -> Self {
        Self {
            issuer_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            tenant_claim: "tenant_id".to_owned(),
            source_type: String::new(),
            external_id_claim: "sub".to_owned(),
            resolve_by: ResolveBy::ExternalId,
            provision_on_login: false,
            default_tenant_id: String::new(),
            extra_ca_cert_path: String::new(),
            hosts: HashMap::new(),
            refresh_enabled: true,
            refresh_safety_margin_seconds: 60,
            refresh_concurrency: 128,
            no_refresh_token_policy: NoRefreshTokenPolicy::Strict,
            refresher_tick_seconds: 5,
            refresh_due_jitter_seconds: 30,
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

/// Audit publishing (PRD `nfr-auth-audit`): the Redpanda sink for auth events.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuditConfig {
    /// Kafka-compatible bootstrap servers (`host:port[,host:port]`). Empty
    /// (default) disables publishing — events stay in the structured log.
    pub brokers: String,
    /// The platform audit topic.
    pub topic: String,
    /// Retention (ms) the authenticator sets on the topic when it creates it,
    /// default **1 day**. NOTE: there is **no consumer** yet — the Audit
    /// Service (`cpt-insightspec-component-be-audit-service`: drain → ClickHouse)
    /// is spec'd but unbuilt, so events are deliberately aged out after this
    /// window (accepted data loss for now). Bump this / drop the bound once the
    /// consumer lands. `0` = don't set retention (leave the cluster default).
    pub retention_ms: u64,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            brokers: String::new(),
            topic: "insight.audit.events".to_owned(),
            retention_ms: 86_400_000, // 1 day — no consumer yet (see field doc)
        }
    }
}

/// Layer-2 rate limiting (DESIGN §4.4, G8): the precise, multi-replica-correct
/// guards behind the gateway's coarse per-IP zone. Buckets key on what
/// identifies the caller (session / OIDC state), never IP. A burst of 0
/// disables that bucket.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RateLimitConfig {
    /// Cap on concurrent live `{asm}:login_state:*` entries; excess
    /// `/auth/login` gets 429 before any state is written (stops a
    /// slow-trickle Redis-exhaustion attack the edge cannot see).
    pub login_state_max: u64,
    /// `/auth/refresh` bucket per session: burst size.
    pub refresh_burst: u32,
    /// `/auth/refresh` bucket per session: sustained refills per minute.
    pub refresh_per_minute: u32,
    /// `/auth/callback` bucket per OIDC `state`: burst size.
    pub callback_burst: u32,
    /// `/auth/callback` bucket per OIDC `state`: sustained refills per minute.
    pub callback_per_minute: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            login_state_max: 1000,
            // The SPA refreshes about once per 8 min; 5-burst + 6/min absorbs
            // multi-tab races and retries with an order of magnitude to spare.
            refresh_burst: 5,
            refresh_per_minute: 6,
            callback_burst: 5,
            callback_per_minute: 10,
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
    /// Requested OIDC scopes. Accepts a YAML list or a space/comma-delimited
    /// string, so a single env override (`APP__…__oidc_scopes`) can set it.
    #[serde(deserialize_with = "de_scopes")]
    pub oidc_scopes: Vec<String>,
    /// Where to send the browser after a successful login when the request
    /// named no (or an unsafe) `return_to`. A site-relative path.
    pub default_return_to: String,
    /// `return_to` prefix honored at login time; must be site-relative and end
    /// in `/`. Empty (default) = any same-origin path. A preview host sets
    /// `/exp/` to confine logins to `/exp/<name>`.
    pub return_to_prefix: String,
    /// Master switch for the preview experiments capability (`/exp/<name>`
    /// tier-3 frontends). Default `false`: a login can never return into the
    /// `/exp/` subtree, so a production stand cannot host experimental
    /// frontends against its data. Dev/demo preview hosts set `true` and serve
    /// experiments over that stand's own data. A per-user RBAC capability will
    /// supersede this environment-level gate.
    pub experiments_enabled: bool,

    // NOTE: first-admin bootstrap (DD-AUTH-08) and RBAC/ACL are deliberately
    // NOT in step 04 — deferred to a separate universe-admin initiative. Local
    // dev seeds the persons table; an unknown person is denied (403). Every
    // session carries `default_roles` only.

    // ── Cross-cutting ────────────────────────────────────────────────────
    /// CSRF `Origin` allowlist (empty = token-required, fail closed).
    pub csrf_origins: Vec<String>,
    /// Janitor pass interval (leader-elected trim of expired index members).
    pub janitor_interval_seconds: u64,
    /// Layer-2 rate limiting knobs (DESIGN §4.4).
    pub rate_limit: RateLimitConfig,
    /// Audit publishing (Redpanda).
    pub audit: AuditConfig,
    /// Back-channel logout: tolerated clock skew on the `logout_token`'s `iat`
    /// (future-dated tokens inside this window are accepted).
    pub backchannel_clock_skew_seconds: u64,
    /// Back-channel logout: how long after `iat` a `logout_token` stays
    /// acceptable. Also sizes the `jti` replay-guard TTL
    /// (`iat + max_age + skew − now`).
    pub backchannel_token_max_age_seconds: u64,
    /// Roles (gateway-JWT `roles` scopes) authorized to call the admin
    /// revoke-by-user operation. The service registry grants one of these to
    /// the services that may force-logout users (e.g. the future permissions
    /// service on grant changes, DD-AUTH-07).
    pub admin_revoke_roles: Vec<String>,
    /// Honor the `__override=<email>` parameter on `/auth/login` (view-as,
    /// #1941): the session is minted for that person instead of the
    /// authenticated one, after a full IdP login. Dev/demo environments ONLY —
    /// the flag marks the whole environment as impersonation-capable, so it
    /// MUST stay `false` anywhere real users log in.
    pub override_enabled: bool,

    // ── Dependencies ─────────────────────────────────────────────────────
    /// Redis connection URL (`redis://host:port`).
    pub redis_url: String,
    /// Directory holding the ES256 signing keys (`current.pem`, optional
    /// `previous.pem`) — a mounted K8s Secret in production.
    pub signing_keys_path: String,
    /// Identity Service base URL for `email -> person_id` resolution.
    pub identity_url: String,

    /// HTTP bind address. Owned by the `api-gateway` host gear; retained for
    /// diagnostics only.
    pub bind_addr: String,

    /// The nested IdP settings.
    pub idp: IdpConfig,

    /// Service-token issuance (§10 G1): the second listener + registry.
    pub service_tokens: ServiceTokensConfig,
}

/// Deserialize `oidc_scopes` from either a YAML list (`["openid","email"]`) or a
/// space/comma-delimited string (`"openid email offline_access"`), so it round-trips
/// through a single env var (`APP__…__oidc_scopes`) — env layers can't express a list.
fn de_scopes<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ListOrStr {
        List(Vec<String>),
        Str(String),
    }
    Ok(match ListOrStr::deserialize(d)? {
        ListOrStr::List(v) => v,
        ListOrStr::Str(s) => s
            .split([' ', ','])
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect(),
    })
}

/// `idp.hosts` deserializes from a native map or a JSON string (the env-var
/// form — env layers can't express nested maps).
fn de_hosts<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<HashMap<String, HostIdpConfig>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum MapOrStr {
        Map(HashMap<String, HostIdpConfig>),
        Str(String),
    }
    match MapOrStr::deserialize(d)? {
        MapOrStr::Map(m) => Ok(m),
        MapOrStr::Str(s) if s.trim().is_empty() => Ok(HashMap::new()),
        MapOrStr::Str(s) => serde_json::from_str(&s)
            .map_err(|e| serde::de::Error::custom(format!("idp.hosts JSON string: {e}"))),
    }
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
            // offline_access omitted: survives-logout token, wrong for a BFF.
            // Add via oidc_scopes for an IdP that needs it (Entra); see insight.yaml.
            oidc_scopes: vec![
                "openid".to_owned(),
                "email".to_owned(),
                "profile".to_owned(),
            ],
            default_return_to: "/".to_owned(),
            return_to_prefix: String::new(),
            experiments_enabled: false,
            csrf_origins: Vec::new(),
            janitor_interval_seconds: 30,
            rate_limit: RateLimitConfig::default(),
            audit: AuditConfig::default(),
            backchannel_clock_skew_seconds: 60,
            backchannel_token_max_age_seconds: 300,
            admin_revoke_roles: vec!["session_admin".to_owned()],
            override_enabled: false,
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
        // intentionally optional — public OIDC clients authenticate with PKCE
        // and no secret. `redis_url` is checked in SessionManager::connect.
        for (name, value) in [
            ("gateway_issuer", &self.gateway_issuer),
            ("redirect_uri", &self.redirect_uri),
            ("signing_keys_path", &self.signing_keys_path),
            ("identity_url", &self.identity_url),
        ] {
            anyhow::ensure!(!value.trim().is_empty(), "{name} is required (empty)");
        }

        // Only the external-id mode reads these. Requiring them in email mode
        // would make an install name a source_type its login never asks about,
        // and a stale value there reads as if it were in force.
        // Provisioning mints by the source-native id the roster observed, and
        // an address is not one — `provisionable_external_id` returns None for
        // every address target. Accepting the pair would leave an operator with
        // the flag on, provisioning off, and nothing but
        // `login_denied_unknown_person` for every person not already seeded.
        anyhow::ensure!(
            !(self.idp.resolve_by == ResolveBy::Email && self.idp.provision_on_login),
            "idp.provision_on_login cannot be used with idp.resolve_by=email — minting needs \
             the source-native id the roster observed, so no login provisions in this mode"
        );

        if self.idp.resolve_by == ResolveBy::ExternalId {
            for (name, value) in [
                ("idp.source_type", &self.idp.source_type),
                ("idp.external_id_claim", &self.idp.external_id_claim),
            ] {
                anyhow::ensure!(
                    !value.trim().is_empty(),
                    "{name} is required (empty) when idp.resolve_by is external_id"
                );
            }
        }

        if self.idp.hosts.is_empty() {
            for (name, value) in [
                ("idp.issuer_url", &self.idp.issuer_url),
                ("idp.client_id", &self.idp.client_id),
            ] {
                anyhow::ensure!(!value.trim().is_empty(), "{name} is required (empty)");
            }
        }
        for (host, entry) in &self.idp.hosts {
            anyhow::ensure!(
                !host.trim().is_empty()
                    && !host.contains('/')
                    && !host.contains(':')
                    && !host.chars().any(char::is_whitespace),
                "idp.hosts key {host:?} must be a bare hostname (no scheme, port, or path)"
            );
            for (name, value) in [
                ("issuer_url", &entry.issuer_url),
                ("client_id", &entry.client_id),
            ] {
                anyhow::ensure!(
                    !value.trim().is_empty(),
                    "idp.hosts.{host}.{name} is required (empty)"
                );
            }
        }

        // `default_return_to` lands verbatim in Location headers (login
        // fallback and every `auth_error` bounce). A non-site-relative value
        // would open-redirect on our own config, and a `#` fragment would hide
        // `auth_error=` from the SPA's query parsing — defeating its login
        // retry loop guard.
        anyhow::ensure!(
            self.default_return_to.starts_with('/')
                && !self.default_return_to.starts_with("//")
                && !self.default_return_to.contains('#')
                && !self.default_return_to.chars().any(char::is_control),
            "default_return_to must be a site-relative path without a fragment"
        );

        // Trailing `/` keeps prefix matching on a path boundary: `/exp/` admits
        // `/exp/<name>`, not `/expunge`.
        let prefix = &self.return_to_prefix;
        anyhow::ensure!(
            prefix.is_empty()
                || (prefix.starts_with('/') && !prefix.starts_with("//") && prefix.ends_with('/')),
            "return_to_prefix {prefix:?} must be a site-relative path prefix ending in '/'"
        );

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

    /// `Default` plus the required fields, so a test can `validate()` in isolation.
    fn valid_config() -> AuthenticatorConfig {
        AuthenticatorConfig {
            gateway_issuer: "https://gw.example".to_owned(),
            redirect_uri: "https://gw.example/auth/callback".to_owned(),
            signing_keys_path: "/keys".to_owned(),
            identity_url: "https://identity.example".to_owned(),
            idp: IdpConfig {
                issuer_url: "https://idp.example".to_owned(),
                client_id: "client".to_owned(),
                // Required since the login bootstrap resolves by external id:
                // `IdpConfig::default()` leaves it empty, so a helper that
                // omitted it would build a config `validate()` refuses.
                source_type: "ms-entra".to_owned(),
                ..IdpConfig::default()
            },
            ..AuthenticatorConfig::default()
        }
    }

    #[test]
    fn return_to_prefix_must_be_site_relative_and_boundaried() {
        for ok in ["", "/exp/"] {
            let cfg = AuthenticatorConfig {
                return_to_prefix: ok.to_owned(),
                ..valid_config()
            };
            assert!(cfg.validate().is_ok(), "should accept prefix {ok:?}");
        }

        for bad in ["/exp", "exp/", "//evil/", "https://evil/"] {
            let cfg = AuthenticatorConfig {
                return_to_prefix: bad.to_owned(),
                ..valid_config()
            };
            assert!(cfg.validate().is_err(), "should reject prefix {bad:?}");
        }
    }

    #[test]
    fn resolve_by_defaults_to_external_id() {
        // The mode is opt-in and never inferred: an install that says nothing
        // keeps the directory-id behaviour it had before the knob existed.
        let idp: IdpConfig = serde_yaml::from_str("source_type: faketest").expect("parses");
        assert_eq!(idp.resolve_by, ResolveBy::ExternalId);

        let idp: IdpConfig =
            serde_yaml::from_str("source_type: faketest\nresolve_by: email").expect("parses");
        assert_eq!(idp.resolve_by, ResolveBy::Email);
    }

    #[test]
    fn every_mode_literal_the_deploy_surfaces_emit_round_trips() {
        // The chart renders `resolve_by: "external_id"` and the gitops script
        // writes the same word; Rust is the last of the three to agree on it.
        // Without this, renaming the serde convention (say to camelCase) leaves
        // `"email"` parsing — it is one lowercase word — while every DEFAULT
        // install fails to boot on a config-deserialize error.
        for (yaml, expected) in [
            ("resolve_by: external_id", ResolveBy::ExternalId),
            ("resolve_by: email", ResolveBy::Email),
        ] {
            let idp: IdpConfig = serde_yaml::from_str(yaml).expect("parses");
            assert_eq!(idp.resolve_by, expected, "{yaml}");
        }

        // A typo must not deserialize into anything. The chart and the script
        // both refuse an unknown word; this is what stops one slipping past
        // them (a hand-set env var, a compose file) into a silent default.
        for bad in [
            "resolve_by: e-mail",
            "resolve_by: External_Id",
            "resolve_by: EMAIL",
        ] {
            assert!(
                serde_yaml::from_str::<IdpConfig>(bad).is_err(),
                "{bad} must not parse",
            );
        }
    }

    #[test]
    fn provisioning_is_refused_in_email_mode_rather_than_silently_inert() {
        let mut cfg = valid_config();
        cfg.idp.resolve_by = ResolveBy::Email;
        cfg.idp.provision_on_login = true;
        assert!(
            cfg.validate().is_err(),
            "the pair must be refused at boot, not discovered as blanket refusals",
        );

        cfg.idp.provision_on_login = false;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn external_id_knobs_are_required_only_in_that_mode() {
        // The default mode resolves through both, so both must be stated.
        let mut cfg = valid_config();
        cfg.idp.source_type = String::new();
        assert!(cfg.validate().is_err(), "source_type required by default");

        let mut cfg = valid_config();
        cfg.idp.external_id_claim = String::new();
        assert!(
            cfg.validate().is_err(),
            "external_id_claim required by default"
        );

        // Email mode consults neither. Requiring them anyway would make an
        // install name a source_type its login never asks about — and a stale
        // value sitting there reads as if it were in force.
        let mut cfg = valid_config();
        cfg.idp.resolve_by = ResolveBy::Email;
        cfg.idp.source_type = String::new();
        cfg.idp.external_id_claim = String::new();
        assert!(
            cfg.validate().is_ok(),
            "email mode needs neither external-id knob"
        );
    }

    #[test]
    fn idp_hosts_parses_native_map_and_json_string_shapes() {
        let yaml = r#"
issuer_url: ""
client_id: ""
source_type: faketest
hosts:
  a.example:
    issuer_url: https://kc.example/realms/a
    client_id: client-a
    client_secret: s3cret
"#;
        let idp: IdpConfig = serde_yaml::from_str(yaml).expect("map shape parses");
        assert_eq!(idp.hosts.len(), 1);
        assert_eq!(
            idp.hosts["a.example"].issuer_url,
            "https://kc.example/realms/a"
        );

        let yaml = r#"
source_type: faketest
hosts: '{"b.example": {"issuer_url": "https://kc.example/realms/b", "client_id": "client-b"}}'
"#;
        let idp: IdpConfig = serde_yaml::from_str(yaml).expect("JSON-string shape parses");
        assert_eq!(idp.hosts["b.example"].client_id, "client-b");

        // Empty string = no map (an unset env override must stay degenerate).
        let idp: IdpConfig = serde_yaml::from_str("hosts: \"\"").expect("empty string parses");
        assert!(idp.hosts.is_empty());

        // A malformed JSON string fails loudly, never silently degenerate.
        assert!(serde_yaml::from_str::<IdpConfig>("hosts: '{broken'").is_err());
    }

    #[test]
    fn flat_issuer_fields_required_only_without_a_hosts_map() {
        let mut cfg = valid_config();
        cfg.idp.issuer_url = String::new();
        assert!(cfg.validate().is_err(), "flat issuer_url required");

        let entry = HostIdpConfig {
            issuer_url: "https://kc.example/realms/a".to_owned(),
            client_id: "client-a".to_owned(),
            ..HostIdpConfig::default()
        };
        let mut cfg = valid_config();
        cfg.idp.issuer_url = String::new();
        cfg.idp.client_id = String::new();
        cfg.idp.hosts = HashMap::from([("a.example".to_owned(), entry.clone())]);
        assert!(
            cfg.validate().is_ok(),
            "map replaces the flat client fields"
        );

        let mut cfg = valid_config();
        cfg.idp.hosts = HashMap::from([(
            "a.example".to_owned(),
            HostIdpConfig {
                issuer_url: String::new(),
                ..entry.clone()
            },
        )]);
        assert!(cfg.validate().is_err(), "entry issuer_url required");

        // Keys must be bare hostnames: no scheme, port, path, or whitespace.
        for bad in ["https://a.example", "a.example:8443", "a.example/x", "a b"] {
            let mut cfg = valid_config();
            cfg.idp.hosts = HashMap::from([(bad.to_owned(), entry.clone())]);
            assert!(cfg.validate().is_err(), "should reject key {bad:?}");
        }
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
