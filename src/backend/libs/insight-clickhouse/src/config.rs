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
    /// Per-query thread cap. Applied as `max_threads` setting.
    /// `None` means the `ClickHouse` server default.
    pub query_max_threads: Option<u32>,
    /// Per-query memory ceiling in bytes. Applied as `max_memory_usage`
    /// setting. `None` means the `ClickHouse` server default.
    pub query_max_memory_bytes: Option<u64>,
}

impl core::fmt::Debug for Config {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Config")
            .field("url", &self.url)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("query_timeout", &self.query_timeout)
            .field("query_max_threads", &self.query_max_threads)
            .field("query_max_memory_bytes", &self.query_max_memory_bytes)
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
            query_max_threads: None,
            query_max_memory_bytes: None,
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

    /// Sets the per-query thread cap.
    #[must_use]
    pub fn with_query_max_threads(mut self, max_threads: u32) -> Self {
        self.query_max_threads = Some(max_threads);
        self
    }

    /// Sets the per-query memory ceiling in bytes.
    #[must_use]
    pub fn with_query_max_memory_bytes(mut self, max_memory_bytes: u64) -> Self {
        self.query_max_memory_bytes = Some(max_memory_bytes);
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
            Config::new("http://ch:8123", "insight").with_query_timeout(Duration::from_mins(1));

        assert_eq!(cfg.query_timeout, Some(Duration::from_mins(1)));
    }

    #[test]
    fn disable_timeout() {
        let cfg = Config::new("http://ch:8123", "insight").without_query_timeout();

        assert!(cfg.query_timeout.is_none());
    }

    #[test]
    fn debug_redacts_the_password_but_keeps_other_fields() {
        let cfg = Config::new("http://ch:8123", "insight").with_auth("admin", "s3cr3t");
        let rendered = format!("{cfg:?}");

        assert!(rendered.contains("<redacted>"), "password must be redacted");
        assert!(
            !rendered.contains("s3cr3t"),
            "raw password must never appear: {rendered}"
        );
        assert!(rendered.contains("admin"), "user is not secret");
        assert!(rendered.contains("http://ch:8123"));
    }
}
