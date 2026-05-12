//! Application configuration.

use figment::Figment;
use figment::providers::{Env, Format, Yaml};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,

    /// `MariaDB` connection URL for the service-owned `identity` database.
    /// Example: `mysql://insight:pass@insight-mariadb:3306/identity`
    pub database_url: String,

    pub clickhouse_url: String,

    #[serde(default = "default_clickhouse_database")]
    pub clickhouse_database: String,

    #[serde(default)]
    pub clickhouse_user: Option<String>,

    #[serde(default)]
    pub clickhouse_password: Option<String>,
}

fn default_bind_addr() -> String {
    "0.0.0.0:8082".to_owned()
}

fn default_clickhouse_database() -> String {
    "insight".to_owned()
}

impl AppConfig {
    /// # Errors
    /// Returns an error if the config file cannot be read or parsed.
    pub fn load(config_path: Option<&str>) -> anyhow::Result<Self> {
        let mut figment = Figment::new();
        if let Some(path) = config_path {
            figment = figment.merge(Yaml::file(path));
        }
        figment = figment.merge(Env::prefixed("IDENTITY__").split("__"));
        let config: Self = figment.extract()?;
        Ok(config)
    }
}
