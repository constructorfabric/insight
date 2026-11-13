use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use toolkit::bootstrap::AppConfig;
use url::Url;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub(crate) struct GearConfig {
    pub(crate) database_url: SecretString,
    pub(crate) database_host: String,
    pub(crate) database_port: u16,
    pub(crate) database_user: String,
    pub(crate) database_name: String,
    pub(crate) database_password: SecretString,
    pub(crate) clickhouse_url: String,
    pub(crate) clickhouse_database: String,
    pub(crate) clickhouse_user: String,
    pub(crate) clickhouse_password: SecretString,
    pub(crate) identity_url: String,
    pub(crate) cluster: ClusterConfig,
    pub(crate) namespace: String,
    pub(crate) tenant_id: String,
    pub(crate) reconcile_workflow_name: String,
}

impl Default for GearConfig {
    fn default() -> Self {
        Self {
            database_url: SecretString::default(),
            database_host: String::new(),
            database_port: 3306,
            database_user: String::new(),
            database_name: "connectors".to_owned(),
            database_password: SecretString::default(),
            clickhouse_url: String::new(),
            clickhouse_database: "insight".to_owned(),
            clickhouse_user: String::new(),
            clickhouse_password: SecretString::default(),
            identity_url: String::new(),
            cluster: ClusterConfig::default(),
            namespace: String::new(),
            tenant_id: String::new(),
            reconcile_workflow_name: String::new(),
        }
    }
}

impl GearConfig {
    pub(crate) fn database_url(&self) -> anyhow::Result<SecretString> {
        if !self.database_url.expose_secret().trim().is_empty() {
            let url = Url::parse(self.database_url.expose_secret())
                .map_err(|_| config_error("database_url", "must be a MySQL URL"))?;
            if url.scheme() != "mysql"
                || url.host_str().is_none()
                || url.path().trim_matches('/').is_empty()
            {
                return Err(config_error("database_url", "must select a MySQL database"));
            }
            return Ok(self.database_url.clone());
        }

        for (key, value) in [
            ("database_host", self.database_host.as_str()),
            ("database_user", self.database_user.as_str()),
            ("database_name", self.database_name.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(config_error(key, "is empty"));
            }
        }
        if self.database_port == 0 {
            return Err(config_error("database_port", "must be greater than zero"));
        }

        let mut url = Url::parse("mysql://localhost")?;
        url.set_host(Some(&self.database_host))
            .map_err(|_| config_error("database_host", "is invalid"))?;
        url.set_port(Some(self.database_port))
            .map_err(|()| config_error("database_port", "is invalid"))?;
        url.set_username(&encode_userinfo(&self.database_user))
            .map_err(|()| config_error("database_user", "is invalid"))?;
        url.set_password(Some(&encode_userinfo(
            self.database_password.expose_secret(),
        )))
        .map_err(|()| config_error("database_password", "is invalid"))?;
        url.path_segments_mut()
            .map_err(|()| config_error("database_name", "is invalid"))?
            .clear()
            .push(&self.database_name);
        Ok(SecretString::from(url.to_string()))
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        self.database_url()?;
        if self.cluster.enabled {
            for (key, value) in [
                ("namespace", &self.namespace),
                ("tenant_id", &self.tenant_id),
            ] {
                if value.is_empty()
                    || !value.bytes().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'.'
                    })
                {
                    return Err(config_error(key, "must be a nonempty cluster identifier"));
                }
            }
        }
        for (key, value) in [
            ("clickhouse_url", self.clickhouse_url.as_str()),
            ("identity_url", self.identity_url.as_str()),
        ] {
            let url = Url::parse(value).map_err(|_| config_error(key, "must be an HTTP(S) URL"))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(config_error(key, "must be an HTTP(S) URL"));
            }
        }
        if self.clickhouse_database.trim().is_empty() {
            return Err(config_error("clickhouse_database", "is empty"));
        }
        Ok(())
    }
}

fn encode_userinfo(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

fn config_error(key: &str, reason: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "gears.connectors.config.{key} {reason} (set APP__gears__connectors__config__{key})"
    )
}

pub(crate) fn extract_gear_config(app: &AppConfig) -> anyhow::Result<GearConfig> {
    let raw = app
        .gears
        .get("connectors")
        .and_then(|gear| gear.get("config"))
        .ok_or_else(|| {
            anyhow::anyhow!("missing `gears.connectors.config` section in configuration")
        })?;
    serde_json::from_value(raw.clone())
        .map_err(|_| anyhow::anyhow!("invalid `gears.connectors.config` configuration"))
}

pub(crate) fn redacted_yaml(config: &AppConfig) -> anyhow::Result<String> {
    let mut redacted = config.clone();
    if let Some(config) = redacted
        .gears
        .get_mut("connectors")
        .and_then(|gear| gear.get_mut("config"))
        .and_then(serde_json::Value::as_object_mut)
    {
        for key in ["database_url", "database_password", "clickhouse_password"] {
            if let Some(secret) = config.get_mut(key) {
                *secret = serde_json::Value::String("<redacted>".to_owned());
            }
        }
    }
    redacted.to_yaml()
}

#[cfg(test)]
mod tests;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ClusterConfig {
    pub(crate) enabled: bool,
}
