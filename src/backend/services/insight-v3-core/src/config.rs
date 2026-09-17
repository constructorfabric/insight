use std::fmt;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use thiserror::Error;

const DEFAULT_CLICKHOUSE_DATABASE: &str = "insight";
const DEFAULT_IDENTITY_DATABASE: &str = "identity";
/// Datasets keep their records apart from the warehouse the rest of Insight
/// builds, so nothing this service creates or drops can reach a table someone
/// else owns.
const DEFAULT_DATASETS_DATABASE: &str = "insight_datasets";
/// How many of a dataset's latest records a reader is shown at once.
const DEFAULT_DATASET_PREVIEW_ROWS: u64 = 50;
/// The widest preview an installation may ask for: the payloads are whole
/// records, so the bound is on what one response may carry, not on taste.
const MAX_DATASET_PREVIEW_ROWS: u64 = 500;
/// The longest an installation may let an abandoned attempt hold a dataset.
const MAX_DATASET_LEASE_SECS: i64 = 3600;
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
    /// The database the datasets' own tables live in.
    pub(crate) datasets_database: String,
    /// How many of a dataset's latest records its page shows.
    pub(crate) dataset_preview_rows: u64,
    /// How long an abandoned create or removal holds its dataset.
    pub(crate) dataset_lease_secs: i64,
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
            datasets_database: DEFAULT_DATASETS_DATABASE.to_owned(),
            dataset_preview_rows: DEFAULT_DATASET_PREVIEW_ROWS,
            dataset_lease_secs: crate::domain::datasets::LEASE_SECS,
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
    datasets_database: String,
    database_url: String,
}

impl StoreConfig {
    pub(crate) fn clickhouse(&self) -> &insight_clickhouse::Client {
        &self.clickhouse
    }

    pub(crate) fn datasets_database(&self) -> &str {
        &self.datasets_database
    }

    pub(crate) fn database_url(&self) -> &str {
        &self.database_url
    }
}

pub(crate) struct ValidatedConfig {
    clickhouse_url: String,
    clickhouse_database: String,
    identity_database: String,
    datasets_database: String,
    dataset_preview_rows: u64,
    dataset_lease_secs: i64,
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
            .field("datasets_database", &self.datasets_database)
            .field("dataset_preview_rows", &self.dataset_preview_rows)
            .field("dataset_lease_secs", &self.dataset_lease_secs)
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
            .field("datasets_database", &self.datasets_database)
            .field("dataset_preview_rows", &self.dataset_preview_rows)
            .field("dataset_lease_secs", &self.dataset_lease_secs)
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

    pub(crate) fn datasets_database(&self) -> String {
        self.datasets_database.clone()
    }

    pub(crate) fn dataset_preview_rows(&self) -> u64 {
        self.dataset_preview_rows
    }

    pub(crate) fn dataset_lease(&self) -> crate::domain::datasets::Lease {
        crate::domain::datasets::Lease::of_seconds(self.dataset_lease_secs)
    }

    /// A client connected to the database the datasets' tables live in, which
    /// is the only database this service creates or drops a table in.
    pub(crate) fn datasets_client(&self) -> insight_clickhouse::Client {
        let mut config =
            insight_clickhouse::Config::new(&self.clickhouse_url, &self.datasets_database);
        if let (Some(user), Some(password)) = (
            self.clickhouse_user.as_deref(),
            self.clickhouse_password.as_ref(),
        ) {
            config = config.with_auth(user, password.expose_secret());
        }

        insight_clickhouse::Client::new(config)
    }
}

impl GearConfig {
    /// What a migration needs: the warehouse it creates tables in, and the
    /// definition store it migrates.
    pub(crate) fn validate_stores(self) -> Result<StoreConfig, ConfigError> {
        require_non_empty("clickhouse_url", &self.clickhouse_url)?;
        require_non_empty("clickhouse_database", &self.clickhouse_database)?;
        validate_datasets_database(&self.datasets_database, &self.clickhouse_database)?;
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
            datasets_database: self.datasets_database,
            database_url: self.database_url,
        })
    }

    pub(crate) fn validate(self) -> Result<ValidatedConfig, ConfigError> {
        require_non_empty("clickhouse_url", &self.clickhouse_url)?;
        require_non_empty("clickhouse_database", &self.clickhouse_database)?;
        require_non_empty("identity_database", &self.identity_database)?;
        validate_datasets_database(&self.datasets_database, &self.clickhouse_database)?;
        if self.dataset_preview_rows == 0 || self.dataset_preview_rows > MAX_DATASET_PREVIEW_ROWS {
            return Err(ConfigError::PreviewRows(MAX_DATASET_PREVIEW_ROWS));
        }
        if self.dataset_lease_secs <= 0 || self.dataset_lease_secs > MAX_DATASET_LEASE_SECS {
            return Err(ConfigError::LeaseSecs(MAX_DATASET_LEASE_SECS));
        }
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
            datasets_database: self.datasets_database,
            dataset_preview_rows: self.dataset_preview_rows,
            dataset_lease_secs: self.dataset_lease_secs,
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

/// The datasets database is interpolated into SQL as a name and is the one
/// place this service creates and drops tables, so it has to be a plain
/// identifier and it has to be a database of its own.
fn validate_datasets_database(datasets: &str, warehouse: &str) -> Result<(), ConfigError> {
    let is_identifier = !datasets.is_empty()
        && datasets
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    if !is_identifier {
        return Err(ConfigError::DatasetsDatabaseName);
    }
    if datasets == warehouse {
        return Err(ConfigError::DatasetsDatabaseIsTheWarehouse);
    }

    Ok(())
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
    #[error("gears.insight-v3-core.config.dataset_preview_rows must be 1 to {0}")]
    PreviewRows(u64),
    #[error("gears.insight-v3-core.config.dataset_lease_secs must be 1 to {0}")]
    LeaseSecs(i64),
    #[error("gears.insight-v3-core.config.datasets_database must be letters, digits or underscore")]
    DatasetsDatabaseName,
    #[error("gears.insight-v3-core.config.datasets_database must not be the clickhouse_database")]
    DatasetsDatabaseIsTheWarehouse,
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
mod tests;
