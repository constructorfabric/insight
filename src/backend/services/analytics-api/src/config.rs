//! Application configuration.

use figment::Figment;
use figment::providers::{Env, Format, Yaml};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    /// HTTP bind address (e.g., `0.0.0.0:8081`).
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,

    /// `MariaDB` connection URL.
    /// Example: `mysql://insight:password@localhost:3306/analytics`
    pub database_url: String,

    /// `ClickHouse` HTTP URL (e.g., `http://localhost:8123`).
    pub clickhouse_url: String,

    /// `ClickHouse` database name (e.g., `insight`).
    #[serde(default = "default_clickhouse_database")]
    pub clickhouse_database: String,

    /// `ClickHouse` username. Optional — omit for no-auth deployments.
    #[serde(default)]
    pub clickhouse_user: Option<String>,

    /// `ClickHouse` password.
    #[serde(default)]
    pub clickhouse_password: Option<String>,

    /// Identity Resolution service base URL (e.g., `http://identity-resolution:8082`).
    /// Optional — when empty, `person_ids` from `$filter` are used directly against
    /// `ClickHouse` without alias resolution (MVP mode).
    #[serde(default)]
    pub identity_resolution_url: String,

    /// Redis URL for caching (e.g., `redis://localhost:6379`).
    #[serde(default)]
    #[allow(dead_code)] // will be used when caching layer is implemented
    pub redis_url: String,
}

fn default_bind_addr() -> String {
    "0.0.0.0:8081".to_owned()
}

fn default_clickhouse_database() -> String {
    "insight".to_owned()
}

impl AppConfig {
    /// Load config: YAML file then environment variables (`ANALYTICS__*`).
    ///
    /// # Errors
    ///
    /// Returns error if config cannot be loaded or parsed.
    pub fn load(config_path: Option<&str>) -> anyhow::Result<Self> {
        let mut figment = Figment::new();

        if let Some(path) = config_path {
            figment = figment.merge(Yaml::file(path));
        }

        figment = figment.merge(Env::prefixed("ANALYTICS__").split("__"));

        let config: Self = figment.extract()?;
        Ok(config)
    }
}
