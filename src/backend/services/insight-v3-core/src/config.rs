use std::fmt;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use thiserror::Error;

const DEFAULT_CLICKHOUSE_DATABASE: &str = "insight";
const DEFAULT_IDENTITY_DATABASE: &str = "identity";
pub(crate) const MIN_INGEST_TOKEN_BYTES: usize = 32;
pub(crate) const MAX_INGEST_TOKEN_BYTES: usize = 1024;
const DEFAULT_CHAT_MODEL: &str = "claude-sonnet-5";
const DEFAULT_MCP_BIND_ADDR: &str = "0.0.0.0:8087";

/// The MCP server's own listener, off unless a deployment asks for it.
///
/// It binds a second port because the gears api-gateway authenticates the REST
/// router against its own audience, which an MCP access token does not carry.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct McpConfig {
    pub(crate) enabled: bool,
    pub(crate) bind_addr: String,
    pub(crate) public_url: String,
    /// Where to fetch the signing keys, when the advertised origin is not
    /// reachable from in here.
    ///
    /// The public URL is the client's view: the audience its token carries and
    /// the resource it discovers. It is not always routable from inside this
    /// process — on a local stand it is `localhost`, which names this container
    /// and not the gateway. Blank derives it from the public URL, which is what
    /// a deployment whose origin routes internally wants.
    pub(crate) jwks_url: String,
    /// Permits an `http` origin on a private network, for a stand without TLS.
    pub(crate) allow_insecure_private_network: bool,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_addr: DEFAULT_MCP_BIND_ADDR.to_owned(),
            public_url: String::new(),
            jwks_url: String::new(),
            allow_insecure_private_network: false,
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub(crate) struct GearConfig {
    pub(crate) clickhouse_url: String,
    pub(crate) clickhouse_database: String,
    /// The database identity materialises into.
    pub(crate) identity_database: String,
    pub(crate) clickhouse_user: Option<String>,
    pub(crate) clickhouse_password: Option<SecretString>,
    /// The read-only principal the assistant's query path connects as. Blank
    /// on a stand without one, and the query path then uses the pair above.
    pub(crate) clickhouse_query_user: Option<String>,
    pub(crate) clickhouse_query_password: Option<SecretString>,
    pub(crate) ingest_token: SecretString,
    pub(crate) anthropic_token: SecretString,
    pub(crate) chat_model: String,
    pub(crate) database_url: String,
    pub(crate) identity_url: String,
    pub(crate) mcp: McpConfig,
}

impl Default for GearConfig {
    fn default() -> Self {
        Self {
            clickhouse_url: String::new(),
            clickhouse_database: DEFAULT_CLICKHOUSE_DATABASE.to_owned(),
            identity_database: DEFAULT_IDENTITY_DATABASE.to_owned(),
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_query_user: None,
            clickhouse_query_password: None,
            ingest_token: SecretString::from(String::new()),
            anthropic_token: SecretString::from(String::new()),
            chat_model: DEFAULT_CHAT_MODEL.to_owned(),
            database_url: String::new(),
            identity_url: String::new(),
            mcp: McpConfig::default(),
        }
    }
}

/// The stores a migration writes. Deliberately not [`ValidatedConfig`]: a
/// migration that cannot serve a request should still run.
pub(crate) struct StoreConfig {
    clickhouse: insight_clickhouse::Client,
    database_url: String,
}

impl StoreConfig {
    pub(crate) fn clickhouse(&self) -> &insight_clickhouse::Client {
        &self.clickhouse
    }

    pub(crate) fn database_url(&self) -> &str {
        &self.database_url
    }
}

pub(crate) struct ValidatedConfig {
    clickhouse_url: String,
    clickhouse_database: String,
    identity_database: String,
    clickhouse_user: Option<String>,
    clickhouse_password: Option<SecretString>,
    clickhouse_query_user: Option<String>,
    clickhouse_query_password: Option<SecretString>,
    ingest_token: IngestToken,
    anthropic_token: SecretString,
    chat_model: String,
    database_url: String,
    identity_url: String,
    mcp: McpConfig,
}

// SAFETY: `database_url` embeds the MariaDB password, so a `?config` in any
// log line must render a marker rather than the value. The secrets beside it
// redact themselves; a plain String does not.
impl fmt::Debug for GearConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const REDACTED: &str = "<redacted>";
        f.debug_struct("GearConfig")
            .field("clickhouse_url", &self.clickhouse_url)
            .field("clickhouse_database", &self.clickhouse_database)
            .field("identity_database", &self.identity_database)
            .field("clickhouse_user", &self.clickhouse_user)
            .field("clickhouse_password", &REDACTED)
            .field("clickhouse_query_user", &self.clickhouse_query_user)
            .field("clickhouse_query_password", &REDACTED)
            .field("ingest_token", &REDACTED)
            .field("anthropic_token", &REDACTED)
            .field("chat_model", &self.chat_model)
            .field("database_url", &REDACTED)
            .field("identity_url", &self.identity_url)
            .field("mcp", &self.mcp)
            .finish()
    }
}

impl fmt::Debug for ValidatedConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const REDACTED: &str = "<redacted>";
        f.debug_struct("ValidatedConfig")
            .field("clickhouse_url", &self.clickhouse_url)
            .field("clickhouse_database", &self.clickhouse_database)
            .field("identity_database", &self.identity_database)
            .field("clickhouse_user", &self.clickhouse_user)
            .field("clickhouse_password", &REDACTED)
            .field("clickhouse_query_user", &self.clickhouse_query_user)
            .field("clickhouse_query_password", &REDACTED)
            .field("ingest_token", &REDACTED)
            .field("anthropic_token", &REDACTED)
            .field("chat_model", &self.chat_model)
            .field("database_url", &REDACTED)
            .field("identity_url", &self.identity_url)
            .field("mcp", &self.mcp)
            .finish()
    }
}

impl ValidatedConfig {
    /// The two stores a migration writes, and nothing else.
    ///
    /// `migrate` creates the raw-data tables in `ClickHouse` and the
    /// definition tables in MariaDB. It never resolves a person, answers a
    /// chat or serves MCP, so holding it to `identity_url`, the chat token or
    /// the MCP origin makes a migration fail for want of a setting it will
    /// not read — as it did on every runner that had no identity service.
    pub(crate) fn stores_from_app_config(
        app: &toolkit::bootstrap::AppConfig,
    ) -> Result<StoreConfig, ConfigLoadError> {
        let raw = app
            .gears
            .get("insight-v3-core")
            .and_then(|gear| gear.get("config"))
            .ok_or(ConfigLoadError::MissingSection)?;
        let config = serde_json::from_value::<GearConfig>(raw.clone())?;

        config.validate_stores().map_err(ConfigLoadError::Invalid)
    }

    pub(crate) fn clickhouse_client(&self) -> insight_clickhouse::Client {
        self.client_as(
            self.clickhouse_user.as_deref(),
            self.clickhouse_password.as_ref(),
        )
    }

    pub(crate) fn clickhouse_query_client(&self) -> insight_clickhouse::Client {
        match (
            self.clickhouse_query_user.as_deref(),
            self.clickhouse_query_password.as_ref(),
        ) {
            (Some(user), Some(password)) => self.client_as(Some(user), Some(password)),
            _ => self.clickhouse_client(),
        }
    }

    fn client_as(
        &self,
        user: Option<&str>,
        password: Option<&SecretString>,
    ) -> insight_clickhouse::Client {
        let mut config =
            insight_clickhouse::Config::new(&self.clickhouse_url, &self.clickhouse_database);
        if let (Some(user), Some(password)) = (user, password) {
            config = config.with_auth(user, password.expose_secret());
        }

        insight_clickhouse::Client::new(config)
    }

    pub(crate) fn ingest_token(&self) -> &SecretString {
        self.ingest_token.as_secret()
    }

    pub(crate) fn anthropic_token(&self) -> &SecretString {
        &self.anthropic_token
    }

    pub(crate) fn chat_model(&self) -> String {
        self.chat_model.clone()
    }

    pub(crate) fn database_url(&self) -> &str {
        &self.database_url
    }

    pub(crate) fn mcp(&self) -> &McpConfig {
        &self.mcp
    }

    pub(crate) fn identity_url(&self) -> &str {
        &self.identity_url
    }

    /// The database gold materialises into — `dbt_project.yml` sets
    /// `gold_database` to the same one this service reads and writes.
    pub(crate) fn clickhouse_database(&self) -> String {
        self.clickhouse_database.clone()
    }

    /// The database holding the names people are known by.
    pub(crate) fn identity_database(&self) -> &str {
        &self.identity_database
    }
}

impl GearConfig {
    /// What a migration needs: the warehouse it creates tables in, and the
    /// definition store it migrates.
    pub(crate) fn validate_stores(self) -> Result<StoreConfig, ConfigError> {
        require_non_empty("clickhouse_url", &self.clickhouse_url)?;
        require_non_empty("clickhouse_database", &self.clickhouse_database)?;
        require_non_empty("database_url", &self.database_url)?;
        validate_credentials(
            self.clickhouse_user.as_deref(),
            self.clickhouse_password.as_ref(),
        )?;

        Ok(StoreConfig {
            clickhouse: insight_clickhouse::Client::new(
                match (
                    self.clickhouse_user.as_deref(),
                    self.clickhouse_password.as_ref(),
                ) {
                    (Some(user), Some(password)) => insight_clickhouse::Config::new(
                        &self.clickhouse_url,
                        &self.clickhouse_database,
                    )
                    .with_auth(user, password.expose_secret()),
                    _ => insight_clickhouse::Config::new(
                        &self.clickhouse_url,
                        &self.clickhouse_database,
                    ),
                },
            ),
            database_url: self.database_url,
        })
    }

    pub(crate) fn validate(self) -> Result<ValidatedConfig, ConfigError> {
        require_non_empty("clickhouse_url", &self.clickhouse_url)?;
        require_non_empty("clickhouse_database", &self.clickhouse_database)?;
        require_non_empty("identity_database", &self.identity_database)?;
        require_non_empty("chat_model", &self.chat_model)?;
        require_non_empty("database_url", &self.database_url)?;
        require_non_empty("identity_url", &self.identity_url)?;
        let ingest_token = IngestToken::parse(self.ingest_token)?;
        validate_credentials(
            self.clickhouse_user.as_deref(),
            self.clickhouse_password.as_ref(),
        )?;
        let clickhouse_query_user = self
            .clickhouse_query_user
            .filter(|user| !user.trim().is_empty());
        let clickhouse_query_password = self
            .clickhouse_query_password
            .filter(|password| !password.expose_secret().trim().is_empty());
        if clickhouse_query_user.is_some() != clickhouse_query_password.is_some() {
            return Err(ConfigError::IncompleteQueryCredentials);
        }
        validate_mcp(&self.mcp)?;

        Ok(ValidatedConfig {
            clickhouse_url: self.clickhouse_url,
            clickhouse_database: self.clickhouse_database,
            identity_database: self.identity_database,
            clickhouse_user: self.clickhouse_user,
            clickhouse_password: self.clickhouse_password,
            clickhouse_query_user,
            clickhouse_query_password,
            ingest_token,
            anthropic_token: self.anthropic_token,
            chat_model: self.chat_model,
            database_url: self.database_url,
            identity_url: self.identity_url,
            mcp: self.mcp,
        })
    }
}

#[derive(Debug)]
struct IngestToken(SecretString);

impl IngestToken {
    fn parse(value: SecretString) -> Result<Self, ConfigError> {
        let exposed = value.expose_secret();
        require_non_empty("ingest_token", exposed)?;
        if exposed.len() < MIN_INGEST_TOKEN_BYTES {
            return Err(ConfigError::IngestTokenTooShort);
        }
        if exposed.len() > MAX_INGEST_TOKEN_BYTES {
            return Err(ConfigError::IngestTokenTooLong);
        }
        if !exposed.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(ConfigError::InvalidIngestTokenCharacters);
        }

        Ok(Self(value))
    }

    fn as_secret(&self) -> &SecretString {
        &self.0
    }
}

fn require_non_empty(field: &'static str, value: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::Empty(field));
    }

    Ok(())
}

fn validate_mcp(mcp: &McpConfig) -> Result<(), ConfigError> {
    if !mcp.enabled {
        return Ok(());
    }

    require_non_empty("mcp.public_url", &mcp.public_url)?;
    mcp.bind_addr
        .parse::<std::net::SocketAddr>()
        .map_err(|_| ConfigError::McpBindAddr)?;

    Ok(())
}

fn validate_credentials(
    user: Option<&str>,
    password: Option<&SecretString>,
) -> Result<(), ConfigError> {
    match (user, password) {
        (None, None) => Ok(()),
        (Some(user), Some(password))
            if !user.trim().is_empty() && !password.expose_secret().is_empty() =>
        {
            Ok(())
        }
        (Some(_), Some(_)) => Err(ConfigError::EmptyCredentials),
        (Some(_), None) | (None, Some(_)) => Err(ConfigError::IncompleteCredentials),
    }
}

#[derive(Debug, Error)]
pub(crate) enum ConfigError {
    #[error("gears.insight-v3-core.config.{0} must not be empty")]
    Empty(&'static str),
    #[error("ClickHouse user and password must both be configured or both omitted")]
    IncompleteCredentials,
    #[error("ClickHouse credentials must not be empty")]
    EmptyCredentials,
    #[error("clickhouse_query_user and clickhouse_query_password must both be set or both empty")]
    IncompleteQueryCredentials,
    #[error("ingest_token must be at least {MIN_INGEST_TOKEN_BYTES} bytes")]
    IngestTokenTooShort,
    #[error("ingest_token must be at most {MAX_INGEST_TOKEN_BYTES} bytes")]
    IngestTokenTooLong,
    #[error("ingest_token must contain only non-whitespace ASCII characters")]
    InvalidIngestTokenCharacters,
    #[error("gears.insight-v3-core.config.mcp.bind_addr is not a socket address")]
    McpBindAddr,
}

#[derive(Debug, Error)]
pub(crate) enum ConfigLoadError {
    #[error("missing gears.insight-v3-core.config section")]
    MissingSection,
    #[error("invalid gears.insight-v3-core.config: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("invalid gears.insight-v3-core.config: {0}")]
    Invalid(#[source] ConfigError),
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::*;

    fn valid_config() -> GearConfig {
        GearConfig {
            clickhouse_url: "http://clickhouse.example.test:8123".to_owned(),
            clickhouse_database: "insight".to_owned(),
            identity_database: "identity".to_owned(),
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_query_user: None,
            clickhouse_query_password: None,
            ingest_token: SecretString::from("test-ingest-token-0123456789abcdef"),
            anthropic_token: SecretString::from("test-anthropic-token"),
            mcp: McpConfig::default(),
            chat_model: "claude-sonnet-5".to_owned(),
            database_url: "mysql://insight:secret@mariadb.example.test:3306/insight_v3".to_owned(),
            identity_url: "http://identity-resolution.example.test:8082".to_owned(),
        }
    }

    #[test]
    fn required_values_must_not_be_empty() {
        for field in ["clickhouse_url", "clickhouse_database", "ingest_token"] {
            let mut config = valid_config();
            match field {
                "clickhouse_url" => config.clickhouse_url.clear(),
                "clickhouse_database" => config.clickhouse_database.clear(),
                "ingest_token" => config.ingest_token = SecretString::from(String::new()),
                _ => unreachable!(),
            }

            assert!(config.validate().is_err(), "empty {field} must be rejected");
        }
    }

    #[test]
    fn chat_model_must_not_be_empty() {
        let mut config = valid_config();
        config.chat_model.clear();

        assert!(config.validate().is_err());
    }

    #[test]
    fn a_stand_without_an_anthropic_token_still_serves_everything_else() {
        let mut config = valid_config();
        config.anthropic_token = SecretString::from(String::new());
        assert!(
            config.validate().is_ok(),
            "the assistant is one endpoint; the rest of the service does not wait on its key"
        );
    }

    #[test]
    fn secrets_are_redacted_from_debug_output() {
        let mut config = valid_config();
        config.clickhouse_user = Some("writer".to_owned());
        config.clickhouse_password = Some(SecretString::from("database-secret"));

        let rendered = format!("{config:?}");

        assert!(!rendered.contains("test-ingest-token-0123456789abcdef"));
        assert!(!rendered.contains("database-secret"));
        assert!(!rendered.contains("test-anthropic-token"));
    }

    #[test]
    fn credentials_must_be_complete() {
        let mut config = valid_config();
        config.clickhouse_user = Some("writer".to_owned());

        assert!(matches!(
            config.validate(),
            Err(ConfigError::IncompleteCredentials)
        ));
    }

    #[test]
    fn ingest_token_must_be_usable_in_the_instance_token_header() {
        let invalid_tokens = [
            "token with space".to_owned(),
            "töken".to_owned(),
            "x".repeat(1025),
        ];

        for token in invalid_tokens {
            let mut config = valid_config();
            config.ingest_token = SecretString::from(token);

            assert!(
                config.validate().is_err(),
                "configured token outside the instance-token header contract must be rejected"
            );
        }
    }

    #[test]
    fn ingest_token_accepts_the_wire_size_boundary() {
        let mut config = valid_config();
        config.ingest_token = SecretString::from("x".repeat(MAX_INGEST_TOKEN_BYTES));

        assert!(config.validate().is_ok());
    }

    #[test]
    fn ingest_token_rejects_values_shorter_than_32_bytes() {
        let mut config = valid_config();
        config.ingest_token = SecretString::from("x".repeat(31));

        assert!(config.validate().is_err());
    }

    #[test]
    fn ingest_token_accepts_the_minimum_size_boundary() {
        let mut config = valid_config();
        config.ingest_token = SecretString::from("x".repeat(32));

        assert!(config.validate().is_ok());
    }

    #[test]
    fn the_query_client_falls_back_to_the_ordinary_credentials_when_no_reader_is_configured() {
        let mut config = valid_config();
        config.clickhouse_user = Some("writer".to_owned());
        config.clickhouse_password = Some(SecretString::from("writer-secret"));

        let validated = config
            .validate()
            .unwrap_or_else(|error| panic!("config must be valid: {error}"));
        let client = validated.clickhouse_query_client();

        assert_eq!(client.config().user.as_deref(), Some("writer"));
        assert_eq!(client.config().password.as_deref(), Some("writer-secret"));
    }

    #[test]
    fn the_query_client_uses_the_reader_when_it_is_configured() {
        let mut config = valid_config();
        config.clickhouse_user = Some("writer".to_owned());
        config.clickhouse_password = Some(SecretString::from("writer-secret"));
        config.clickhouse_query_user = Some("reader".to_owned());
        config.clickhouse_query_password = Some(SecretString::from("reader-secret"));

        let validated = config
            .validate()
            .unwrap_or_else(|error| panic!("config must be valid: {error}"));
        let client = validated.clickhouse_query_client();

        assert_eq!(client.config().user.as_deref(), Some("reader"));
        assert_eq!(client.config().password.as_deref(), Some("reader-secret"));
    }

    #[test]
    fn a_blank_reader_setting_reads_as_unset_rather_than_as_a_credential() {
        let mut config = valid_config();
        config.clickhouse_user = Some("writer".to_owned());
        config.clickhouse_password = Some(SecretString::from("writer-secret"));
        config.clickhouse_query_user = Some(String::new());
        config.clickhouse_query_password = Some(SecretString::from(String::new()));

        let validated = config
            .validate()
            .unwrap_or_else(|error| panic!("config must be valid: {error}"));
        let client = validated.clickhouse_query_client();

        assert_eq!(client.config().user.as_deref(), Some("writer"));
    }

    #[test]
    fn half_a_reader_credential_is_refused_instead_of_falling_back() {
        let mut config = valid_config();
        config.clickhouse_query_user = Some("reader".to_owned());

        assert!(matches!(
            config.validate(),
            Err(ConfigError::IncompleteQueryCredentials)
        ));
    }
    fn mcp(enabled: bool, bind_addr: &str, public_url: &str) -> McpConfig {
        McpConfig {
            enabled,
            bind_addr: bind_addr.to_owned(),
            public_url: public_url.to_owned(),
            jwks_url: String::new(),
            allow_insecure_private_network: false,
        }
    }

    #[test]
    fn the_mcp_server_is_off_and_bound_to_the_documented_port_by_default() {
        let config = McpConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.bind_addr, "0.0.0.0:8087");
        assert!(config.public_url.is_empty());
    }

    #[test]
    fn a_disabled_mcp_server_needs_no_public_url() {
        assert!(validate_mcp(&McpConfig::default()).is_ok());
    }

    #[test]
    fn an_enabled_mcp_server_without_a_public_url_is_refused() {
        let Err(error) = validate_mcp(&mcp(true, "0.0.0.0:8087", "")) else {
            panic!("a server with no origin has no issuer to verify a token against");
        };

        assert!(
            matches!(error, ConfigError::Empty("mcp.public_url")),
            "{error:?}"
        );
    }

    #[test]
    fn an_enabled_mcp_server_with_an_unparseable_bind_address_is_refused() {
        let Err(error) = validate_mcp(&mcp(
            true,
            "not-an-address",
            "https://insight.example.invalid",
        )) else {
            panic!("an address that is not a socket address cannot be bound");
        };

        assert!(matches!(error, ConfigError::McpBindAddr), "{error:?}");
    }

    #[test]
    fn an_enabled_mcp_server_is_accepted_with_an_origin_and_an_address() {
        assert!(
            validate_mcp(&mcp(
                true,
                "0.0.0.0:8087",
                "https://insight.example.invalid"
            ))
            .is_ok()
        );
    }

    #[test]
    fn a_config_whose_mcp_section_is_invalid_is_refused_as_a_whole() {
        let mut raw = valid_config();
        raw.mcp = mcp(true, "0.0.0.0:8087", "");

        let Err(error) = raw.validate() else {
            panic!("an enabled server with no origin makes the whole config invalid");
        };

        assert!(
            matches!(error, ConfigError::Empty("mcp.public_url")),
            "{error:?}"
        );
    }
}
