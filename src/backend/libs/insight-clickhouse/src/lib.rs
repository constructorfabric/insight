//! Shared `ClickHouse` client for Insight backend services.
//!
//! Provides:
//! - [`Client`] — configured `ClickHouse` connection with tenant-scoped queries
//! - [`QueryBuilder`] — parameterized query builder (no string interpolation)
//! - [`Config`] — connection configuration
//! - [`Error`] — error types
//!
//! # Usage
//!
//! ```rust,ignore
//! use insight_clickhouse::{Client, Config};
//! use uuid::Uuid;
//!
//! let config = Config::new("http://localhost:8123", "insight");
//! let client = Client::new(config);
//!
//! let tenant_id = Uuid::now_v7();
//! let rows = client
//!     .query("SELECT ?fields FROM gold.pr_cycle_time WHERE tenant_id = ?")
//!     .bind(tenant_id)
//!     .fetch_all::<PrCycleTime>()
//!     .await?;
//! ```

pub mod config;
pub mod error;
pub mod query;

pub use config::Config;
pub use error::Error;
pub use query::QueryBuilder;

use clickhouse::Client as ChClient;

/// `ClickHouse` client wrapper with Insight-specific defaults.
///
/// Wraps the `clickhouse` crate client with:
/// - Preconfigured database and URL from [`Config`]
/// - Query timeout enforcement
/// - Tenant-scoped query builder via [`tenant_query`]
#[derive(Clone)]
pub struct Client {
    inner: ChClient,
    config: Config,
}

impl Client {
    /// Creates a new client from configuration.
    #[must_use]
    pub fn new(config: Config) -> Self {
        let mut inner = ChClient::default()
            .with_url(&config.url)
            .with_database(&config.database);

        if let Some(user) = &config.user {
            inner = inner.with_user(user);
        }
        if let Some(password) = &config.password {
            inner = inner.with_password(password);
        }

        Self { inner, config }
    }

    /// Returns a raw query handle for the given SQL.
    ///
    /// Use bind parameters (`?`) for all user-supplied values.
    /// **Never** interpolate values into the SQL string.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the query fails.
    pub fn query(&self, sql: &str) -> clickhouse::query::Query {
        let mut q = self.inner.query(sql);
        if let Some(timeout) = self.config.query_timeout {
            q = q.with_option("max_execution_time", timeout.as_secs().to_string());
        }
        if let Some(max_threads) = self.config.query_max_threads {
            q = q.with_option("max_threads", max_threads.to_string());
        }
        if let Some(max_memory_bytes) = self.config.query_max_memory_bytes {
            q = q.with_option("max_memory_usage", max_memory_bytes.to_string());
        }
        q
    }

    /// Returns a [`QueryBuilder`] scoped to the given tenant.
    ///
    /// The builder automatically adds `WHERE insight_tenant_id = ?` and binds the
    /// tenant ID. All subsequent filters are appended with `AND`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] if the table name contains unsafe characters.
    pub fn tenant_query(&self, table: &str, tenant_id: uuid::Uuid) -> Result<QueryBuilder, Error> {
        QueryBuilder::new(self.clone(), table, tenant_id)
    }

    /// Returns the underlying `clickhouse` crate client for advanced usage.
    #[must_use]
    pub fn inner(&self) -> &ChClient {
        &self.inner
    }

    /// Returns the current configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use uuid::Uuid;

    // All tests here are connection-free: `Client::new` and `query` only
    // configure the underlying handle / build a lazy query — nothing opens a
    // socket. The execution paths (`fetch_all`) require a live ClickHouse and
    // are exercised elsewhere.

    #[test]
    fn new_applies_auth_credentials() {
        // Covers the `user` + `password` branches in `Client::new`.
        let client = Client::new(
            Config::new("http://localhost:8123", "insight").with_auth("admin", "s3cr3t"),
        );
        assert_eq!(client.config().url, "http://localhost:8123");
        assert_eq!(client.config().database, "insight");
        assert_eq!(client.config().user.as_deref(), Some("admin"));
        assert_eq!(client.config().password.as_deref(), Some("s3cr3t"));
    }

    #[test]
    fn new_without_auth_leaves_credentials_unset() {
        let client = Client::new(Config::new("http://ch:8123", "insight"));
        assert!(client.config().user.is_none());
        assert!(client.config().password.is_none());
    }

    #[test]
    fn query_with_timeout_builds_a_handle() {
        // Default config carries a 30s timeout, so the `max_execution_time`
        // option branch runs.
        let client = Client::new(Config::new("http://localhost:8123", "insight"));
        let _q = client.query("SELECT 1");
    }

    #[test]
    fn query_without_timeout_skips_the_option() {
        // `without_query_timeout` -> the `None` branch in `query`.
        let client =
            Client::new(Config::new("http://localhost:8123", "insight").without_query_timeout());
        let _q = client.query("SELECT 1");
    }

    #[test]
    fn query_honours_a_custom_timeout() {
        let client = Client::new(
            Config::new("http://localhost:8123", "insight")
                .with_query_timeout(Duration::from_secs(5)),
        );
        let _q = client.query("SELECT 1");
    }

    #[test]
    fn query_applies_thread_and_memory_bounds() {
        // Both `Some` branches in `query` run.
        let client = Client::new(
            Config::new("http://localhost:8123", "insight")
                .with_query_max_threads(4)
                .with_query_max_memory_bytes(1_610_612_736),
        );
        let _q = client.query("SELECT 1");
    }

    #[test]
    fn inner_exposes_the_raw_handle() {
        let client = Client::new(Config::new("http://localhost:8123", "insight"));
        let _raw = client.inner();
    }

    #[test]
    fn tenant_query_scopes_to_the_table() -> Result<(), Error> {
        let client = Client::new(Config::new("http://localhost:8123", "insight"));
        let sql = client
            .tenant_query("gold.pr_cycle_time", Uuid::nil())?
            .to_sql();
        assert!(sql.contains("insight_tenant_id"));
        Ok(())
    }

    #[test]
    fn tenant_query_rejects_an_unsafe_table_name() {
        let client = Client::new(Config::new("http://localhost:8123", "insight"));
        let err = client.tenant_query("gold.pr; DROP TABLE x", Uuid::nil());
        assert!(matches!(err, Err(Error::InvalidQuery(_))));
    }
}
