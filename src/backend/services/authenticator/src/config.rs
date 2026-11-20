use std::collections::HashMap;

use serde::Deserialize;

const REDACTED_SECRET: &str = "<redacted>";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoRefreshTokenPolicy {
    Strict,
    LoginOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveBy {
    ExternalId,
    Email,
}

#[derive(Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HostIdpConfig {
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub default_tenant_id: String,
}

impl std::fmt::Debug for HostIdpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostIdpConfig")
            .field("issuer_url", &self.issuer_url)
            .field("client_id", &self.client_id)
            .field("client_secret", &REDACTED_SECRET)
            .field("redirect_uri", &self.redirect_uri)
            .field("default_tenant_id", &self.default_tenant_id)
            .finish()
    }
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IdpConfig {
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub tenant_claim: String,
    pub source_type: String,
    /// The generic OIDC `sub` is NOT the stable directory id for directory-backed
    /// IdPs (Entra: `oid`).
    pub external_id_claim: String,
    pub resolve_by: ResolveBy,
    pub provision_on_login: bool,
    // SAFETY: tenancy shortcut (#1687, #1782): a token with no tenant claim is given this tenant.
    pub default_tenant_id: String,
    pub extra_ca_cert_path: String,
    #[serde(deserialize_with = "de_hosts")]
    pub hosts: HashMap<String, HostIdpConfig>,
    pub refresh_enabled: bool,
    pub refresh_safety_margin_seconds: u64,
    pub refresh_concurrency: u32,
    pub no_refresh_token_policy: NoRefreshTokenPolicy,
    pub refresher_tick_seconds: u64,
    pub refresh_due_jitter_seconds: u64,
}

impl std::fmt::Debug for IdpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdpConfig")
            .field("issuer_url", &self.issuer_url)
            .field("client_id", &self.client_id)
            .field("client_secret", &REDACTED_SECRET)
            .field("tenant_claim", &self.tenant_claim)
            .field("source_type", &self.source_type)
            .field("external_id_claim", &self.external_id_claim)
            .field("resolve_by", &self.resolve_by)
            .field("provision_on_login", &self.provision_on_login)
            .field("default_tenant_id", &self.default_tenant_id)
            .field("extra_ca_cert_path", &self.extra_ca_cert_path)
            .field("hosts", &self.hosts)
            .field("refresh_enabled", &self.refresh_enabled)
            .field(
                "refresh_safety_margin_seconds",
                &self.refresh_safety_margin_seconds,
            )
            .field("refresh_concurrency", &self.refresh_concurrency)
            .field("no_refresh_token_policy", &self.no_refresh_token_policy)
            .field("refresher_tick_seconds", &self.refresher_tick_seconds)
            .field(
                "refresh_due_jitter_seconds",
                &self.refresh_due_jitter_seconds,
            )
            .finish()
    }
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

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ServiceRegistryEntry {
    pub public_keys: Vec<String>,
    pub public_key_paths: Vec<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServiceTokensConfig {
    pub token_bind_addr: String,
    pub audience: String,
    pub assertion_max_lifetime_seconds: u64,
    pub token_ttl_seconds: u64,
    pub clock_skew_leeway_seconds: u64,
    pub public_key_dir: String,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuditConfig {
    pub brokers: String,
    pub topic: String,
    pub retention_ms: u64,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            brokers: String::new(),
            topic: "insight.audit.events".to_owned(),
            retention_ms: 86_400_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RateLimitConfig {
    pub login_state_max: u64,
    pub refresh_burst: u32,
    pub refresh_per_minute: u32,
    pub callback_burst: u32,
    pub callback_per_minute: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            login_state_max: 1000,
            refresh_burst: 5,
            refresh_per_minute: 6,
            callback_burst: 5,
            callback_per_minute: 10,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpOAuthConfig {
    pub enabled: bool,
    pub public_url: String,
    pub allow_insecure_private_network: bool,
    pub authorization_code_ttl_seconds: u64,
    pub access_token_ttl_seconds: u64,
}

impl Default for McpOAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            public_url: String::new(),
            allow_insecure_private_network: false,
            authorization_code_ttl_seconds: 300,
            access_token_ttl_seconds: 600,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthenticatorConfig {
    pub session_ttl_seconds: u64,
    pub session_absolute_lifetime_seconds: u64,
    pub session_refresh_safety_margin_seconds: u64,
    pub refresh_jitter_seconds: u64,
    pub refresh_grace_ms: u64,

    pub jwt_ttl_seconds: u64,
    pub jwt_reissue_after_seconds: u64,
    pub default_roles: Vec<String>,
    pub authz_cache_max_age_seconds: u64,
    pub gateway_issuer: String,
    pub jwt_audience: String,

    pub redirect_uri: String,
    #[serde(deserialize_with = "de_scopes")]
    pub oidc_scopes: Vec<String>,
    pub default_return_to: String,
    pub return_to_prefix: String,
    pub experiments_enabled: bool,

    pub csrf_origins: Vec<String>,
    pub janitor_interval_seconds: u64,
    pub rate_limit: RateLimitConfig,
    pub audit: AuditConfig,
    pub backchannel_clock_skew_seconds: u64,
    pub backchannel_token_max_age_seconds: u64,
    pub admin_revoke_roles: Vec<String>,
    pub override_enabled: bool,

    pub redis_url: String,
    pub signing_keys_path: String,
    pub identity_url: String,

    pub bind_addr: String,

    pub idp: IdpConfig,

    pub service_tokens: ServiceTokensConfig,

    pub mcp_oauth: McpOAuthConfig,
}

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
            mcp_oauth: McpOAuthConfig::default(),
        }
    }
}

impl AuthenticatorConfig {
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

        validate_required_fields(self)?;

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

        anyhow::ensure!(
            self.default_return_to.starts_with('/')
                && !self.default_return_to.starts_with("//")
                && !self.default_return_to.contains('#')
                && !self.default_return_to.chars().any(char::is_control),
            "default_return_to must be a site-relative path without a fragment"
        );

        let prefix = &self.return_to_prefix;
        anyhow::ensure!(
            prefix.is_empty()
                || (prefix.starts_with('/') && !prefix.starts_with("//") && prefix.ends_with('/')),
            "return_to_prefix {prefix:?} must be a site-relative path prefix ending in '/'"
        );

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

        validate_mcp_oauth(&self.mcp_oauth)?;
        Ok(())
    }
}

fn validate_required_fields(config: &AuthenticatorConfig) -> anyhow::Result<()> {
    for (name, value) in [
        ("gateway_issuer", &config.gateway_issuer),
        ("redirect_uri", &config.redirect_uri),
        ("signing_keys_path", &config.signing_keys_path),
        ("identity_url", &config.identity_url),
    ] {
        anyhow::ensure!(!value.trim().is_empty(), "{name} is required (empty)");
    }
    Ok(())
}

fn validate_mcp_oauth(config: &McpOAuthConfig) -> anyhow::Result<()> {
    if !config.enabled {
        return Ok(());
    }

    let public_url = url::Url::parse(&config.public_url)
        .map_err(|error| anyhow::anyhow!("mcp_oauth.public_url is invalid: {error}"))?;
    let private_http = public_url.scheme() == "http"
        && config.allow_insecure_private_network
        && public_url
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|address| match address {
                std::net::IpAddr::V4(address) => address.is_private(),
                std::net::IpAddr::V6(address) => (address.segments()[0] & 0xfe00) == 0xfc00,
            });
    anyhow::ensure!(
        matches!(public_url.scheme(), "https" | "http")
            && (public_url.scheme() == "https"
                || matches!(
                    public_url.host_str(),
                    Some("localhost" | "127.0.0.1" | "::1")
                )
                || private_http)
            && public_url.path() == "/"
            && public_url.query().is_none()
            && public_url.fragment().is_none()
            && public_url.username().is_empty()
            && public_url.password().is_none(),
        "mcp_oauth.public_url must be an HTTPS origin or an allowed local HTTP origin"
    );
    anyhow::ensure!(
        config.authorization_code_ttl_seconds > 0,
        "mcp_oauth.authorization_code_ttl_seconds must be > 0"
    );
    anyhow::ensure!(
        config.access_token_ttl_seconds > 0,
        "mcp_oauth.access_token_ttl_seconds must be > 0"
    );
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

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

    fn valid_config() -> AuthenticatorConfig {
        AuthenticatorConfig {
            gateway_issuer: "https://gw.example".to_owned(),
            redirect_uri: "https://gw.example/auth/callback".to_owned(),
            signing_keys_path: "/keys".to_owned(),
            identity_url: "https://identity.example".to_owned(),
            idp: IdpConfig {
                issuer_url: "https://idp.example".to_owned(),
                client_id: "client".to_owned(),
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
        let idp: IdpConfig = serde_yaml::from_str("source_type: faketest").expect("parses");
        assert_eq!(idp.resolve_by, ResolveBy::ExternalId);

        let idp: IdpConfig =
            serde_yaml::from_str("source_type: faketest\nresolve_by: email").expect("parses");
        assert_eq!(idp.resolve_by, ResolveBy::Email);
    }

    #[test]
    fn every_mode_literal_the_deploy_surfaces_emit_round_trips() {
        for (yaml, expected) in [
            ("resolve_by: external_id", ResolveBy::ExternalId),
            ("resolve_by: email", ResolveBy::Email),
        ] {
            let idp: IdpConfig = serde_yaml::from_str(yaml).expect("parses");
            assert_eq!(idp.resolve_by, expected, "{yaml}");
        }

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
        let mut cfg = valid_config();
        cfg.idp.source_type = String::new();
        assert!(cfg.validate().is_err(), "source_type required by default");

        let mut cfg = valid_config();
        cfg.idp.external_id_claim = String::new();
        assert!(
            cfg.validate().is_err(),
            "external_id_claim required by default"
        );

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

        let idp: IdpConfig = serde_yaml::from_str("hosts: \"\"").expect("empty string parses");
        assert!(idp.hosts.is_empty());

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

        for bad in ["https://a.example", "a.example:8443", "a.example/x", "a b"] {
            let mut cfg = valid_config();
            cfg.idp.hosts = HashMap::from([(bad.to_owned(), entry.clone())]);
            assert!(cfg.validate().is_err(), "should reject key {bad:?}");
        }
    }

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
