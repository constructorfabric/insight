//! `ClickHouse` connection configuration.

use std::time::Duration;

/// `ClickHouse` connection configuration.
///
/// `Debug` impl redacts the password field.
#[derive(Clone)]
pub struct Config {
    /// `ClickHouse` HTTP URL (e.g., `http://localhost:8123`).
    pub url: String,
    /// Database name (e.g., `insight`).
    pub database: String,
    /// Optional username.
    pub user: Option<String>,
    /// Optional password.
    pub password: Option<String>,
    /// Per-query timeout. Applied as `max_execution_time` setting.
    /// `None` means no timeout (`ClickHouse` server default).
    pub query_timeout: Option<Duration>,
}

impl core::fmt::Debug for Config {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Config")
            .field("url", &self.url)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("query_timeout", &self.query_timeout)
            .finish()
    }
}

impl Config {
    /// Creates a new config with the given URL and database.
    ///
    /// Defaults: no auth, 30-second query timeout.
    #[must_use]
    pub fn new(url: impl Into<String>, database: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            database: database.into(),
            user: None,
            password: None,
            query_timeout: Some(Duration::from_secs(30)),
        }
    }

    /// Sets authentication credentials.
    #[must_use]
    pub fn with_auth(mut self, user: impl Into<String>, password: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self.password = Some(password.into());
        self
    }

    /// Sets the per-query timeout.
    #[must_use]
    pub fn with_query_timeout(mut self, timeout: Duration) -> Self {
        self.query_timeout = Some(timeout);
        self
    }

    /// Disables the per-query timeout.
    #[must_use]
    pub fn without_query_timeout(mut self) -> Self {
        self.query_timeout = None;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_30s_timeout() {
        let cfg = Config::new("http://localhost:8123", "test_db");

        assert_eq!(cfg.url, "http://localhost:8123");
        assert_eq!(cfg.database, "test_db");
        assert!(cfg.user.is_none());
        assert!(cfg.password.is_none());
        assert_eq!(cfg.query_timeout, Some(Duration::from_secs(30)));
    }

    #[test]
    fn with_auth_sets_credentials() {
        let cfg = Config::new("http://ch:8123", "insight").with_auth("admin", "secret");

        assert_eq!(cfg.user.as_deref(), Some("admin"));
        assert_eq!(cfg.password.as_deref(), Some("secret"));
    }

    #[test]
    fn custom_timeout() {
        let cfg =
            Config::new("http://ch:8123", "insight").with_query_timeout(Duration::from_secs(60));

        assert_eq!(cfg.query_timeout, Some(Duration::from_secs(60)));
    }

    #[test]
    fn disable_timeout() {
        let cfg = Config::new("http://ch:8123", "insight").without_query_timeout();

        assert!(cfg.query_timeout.is_none());
    }
}
