//! Shared `ClickHouse` client for Insight backend services.
//!
//! Provides:
//! - [`Client`] — configured `ClickHouse` connection with tenant-scoped queries
//! - [`QueryBuilder`] — parameterized query builder (no string interpolation)
//! - [`Config`] — connection configuration
//! - [`Topology`] — whether the target `ClickHouse` replicates
//! - [`Ddl`] / [`TableSpec`] — the one place data-layer statements are written
//! - [`MissingRelation`] — the one reading of "the warehouse has no such relation"
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
mod consistency;
pub mod ddl;
pub mod error;
pub mod missing_relation;
pub mod query;
pub mod topology;

pub use config::Config;
pub use ddl::{Ddl, TableSpec};
pub use error::Error;
pub use missing_relation::MissingRelation;
pub use query::QueryBuilder;
pub use topology::Topology;

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
            q = q.with_setting("max_execution_time", timeout.as_secs().to_string());
        }
        if let Some(max_threads) = self.config.query_max_threads {
            q = q.with_setting("max_threads", max_threads.to_string());
        }
        if let Some(max_memory_bytes) = self.config.query_max_memory_bytes {
            q = q.with_setting("max_memory_usage", max_memory_bytes.to_string());
        }
        q
    }

    /// Returns a query whose answer includes every row a
    /// [`consistent_insert`](Self::consistent_insert) on this warehouse has
    /// confirmed — the read half of a sequence whose next decision depends on
    /// this caller's own write.
    ///
    /// Identical to [`query`](Self::query) on a standalone warehouse.
    pub fn consistent_query(&self, sql: &str) -> clickhouse::query::Query {
        let mut query = self.query(sql);
        for (name, value) in consistency::read_settings(&self.config.topology) {
            query = query.with_setting(*name, *value);
        }
        query
    }

    /// Returns an insert a following [`consistent_query`](Self::consistent_query)
    /// is guaranteed to see.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the insert handle cannot be opened.
    pub async fn consistent_insert<T: clickhouse::Row>(
        &self,
        table: &str,
    ) -> Result<clickhouse::insert::Insert<T>, Error> {
        let mut insert = self.inner.insert::<T>(table).await?;
        for (name, value) in consistency::write_settings(&self.config.topology) {
            insert = insert.with_setting(*name, *value);
        }
        Ok(insert)
    }

    /// Returns the statement `ddl` renders to, once this connection has
    /// agreed that it is its own to run.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] if the statement names a database other
    /// than the one this connection is scoped to — a service changes the data
    /// layer in its own database and nowhere else.
    pub fn ddl_sql(&self, ddl: &Ddl) -> Result<String, Error> {
        if ddl.database() != self.config.database {
            tracing::warn!(
                statement = %ddl.database(),
                own = %self.config.database,
                "rejected DDL aimed outside this connection's database"
            );
            return Err(Error::InvalidQuery(format!(
                "this connection owns `{}`, so it cannot change `{}`",
                self.config.database,
                ddl.database()
            )));
        }

        Ok(ddl.render())
    }

    /// Runs `ddl` against the warehouse.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the statement is not this connection's to run, or
    /// if the warehouse refuses it.
    pub async fn execute_ddl(&self, ddl: &Ddl) -> Result<(), Error> {
        let sql = self.ddl_sql(ddl)?;
        tracing::debug!(sql = %sql, "running data-layer DDL");
        self.query(&sql).execute().await?;
        Ok(())
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

    fn create_in(database: &str) -> Ddl {
        Ddl::create_table(TableSpec::merge_tree(
            database,
            "records",
            &[("id", "UUID")],
            &["id"],
        ))
    }

    #[test]
    fn a_statement_carries_no_cluster_clause_on_either_topology() -> Result<(), Error> {
        // The `Replicated` database engine replays every DDL inside it, so a
        // clause here would be a second answer to the same question.
        for topology in [Topology::Standalone, Topology::new(true, "insight_cluster")] {
            let client =
                Client::new(Config::new("http://ch:8123", "identity").with_topology(topology));

            let sql = client.ddl_sql(&create_in("identity"))?;

            assert!(!sql.contains("ON CLUSTER"), "{sql}");
            assert!(!sql.contains("ReplicatedMergeTree"), "{sql}");
        }
        Ok(())
    }

    #[test]
    fn every_statement_aimed_at_another_service_s_database_is_refused() {
        let client = Client::new(Config::new("http://ch:8123", "insight_datasets"));

        let refused = [
            create_in("silver"),
            Ddl::drop_table("silver", "class_commits"),
            Ddl::exchange_tables("silver", "a", "b"),
            Ddl::rename_table("silver", "a", "b"),
        ];

        for statement in refused {
            assert!(
                matches!(client.ddl_sql(&statement), Err(Error::InvalidQuery(_))),
                "a service changes its own database and nowhere else: {statement:?}"
            );
        }
    }

    #[test]
    fn a_consistent_query_builds_a_handle_on_either_topology() {
        for topology in [Topology::Standalone, Topology::new(true, "insight_cluster")] {
            let client =
                Client::new(Config::new("http://ch:8123", "identity").with_topology(topology));
            let _q = client.consistent_query("SELECT count() FROM identity.identity_persons");
        }
    }
}
