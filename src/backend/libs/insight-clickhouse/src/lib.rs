//! Shared `ClickHouse` client for Insight backend services.
//!
//! Provides:
//! - [`Client`] — configured `ClickHouse` connection with tenant-scoped queries
//! - [`QueryBuilder`] — parameterized query builder (no string interpolation)
//! - [`Config`] — connection configuration
//! - [`Topology`] — whether the tables a service creates replicate
//! - [`DDL_TASK_TIMEOUT`] — how long a coordinated statement waits for the replicas
//! - [`Unfinished`] — the one reading of "the warehouse stopped waiting on a statement"
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
mod coordination;
pub mod ddl;
pub mod error;
pub mod missing_relation;
pub mod query;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod topology;

pub use config::Config;
pub use coordination::{DDL_TASK_TIMEOUT, Unfinished};
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
        self.bounded(self.inner.query(sql))
    }

    fn bounded(&self, mut q: clickhouse::query::Query) -> clickhouse::query::Query {
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
        for (name, value) in consistency::read_settings(self.config.topology) {
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
        for (name, value) in consistency::write_settings(self.config.topology) {
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

        Ok(ddl.render(self.config.topology))
    }

    /// Refuses to work against `database` unless it stands with an engine
    /// that agrees with this connection's topology.
    ///
    /// Nothing in a service creates a database: `create-databases.sh` is the
    /// one site, so a missing one is reported with that remedy. A plain
    /// database under a replicated topology would hold every table on one
    /// replica; a `Replicated` one under a standalone topology would take
    /// engines that disagree table by table.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Topology`] with the remedy, or [`Error::Clickhouse`]
    /// if `system.databases` could not be read.
    pub async fn assert_database_topology(&self, database: &str) -> Result<(), Error> {
        // INVARIANT: probed from `system`, not the connection's own database:
        // the server rejects any query sent in the name of a database that
        // does not exist, which is the first case this check reports.
        let probe = self.inner.clone().with_database("system");
        let engines: Vec<String> = self
            .bounded(probe.query("SELECT engine FROM system.databases WHERE name = ?"))
            .bind(database)
            .fetch_all()
            .await?;
        let Some(engine) = engines.into_iter().next() else {
            return Err(Error::Topology(format!(
                "database `{database}` does not exist, and this service never creates one.\n  \
                 Run src/ingestion/scripts/create-databases.sh (the chart's clickhouse-databases \
                 hook; clickhouse.createDatabases) and restart."
            )));
        };

        match (self.config.topology.is_replicated(), engine == "Replicated") {
            (true, true) | (false, false) => Ok(()),
            (true, false) => Err(Error::Topology(format!(
                "clickhouse_cluster_mode is on, but database `{database}` has engine `{engine}`; \
                 every table created in it would reach one replica only.\n  \
                 Turning the flag on does not convert it: convert the database first \
                 (docs/operations/CLICKHOUSE-CLUSTER-CONVERSION.md, \
                 convert-database-to-cluster.sh), then restart."
            ))),
            (false, true) => Err(Error::Topology(format!(
                "database `{database}` is Replicated, but clickhouse_cluster_mode is off; \
                 this service would emit standalone engines into it.\n  \
                 Turn clickhouse.clusterMode back on (with clickhouse.clusterName), or convert \
                 the database back to a plain one first \
                 (docs/operations/CLICKHOUSE-CLUSTER-CONVERSION.md, \"Rollback\")."
            ))),
        }
    }

    /// Runs `ddl` against the warehouse.
    ///
    /// On a replicated warehouse the statement waits for the active replicas
    /// only, for at most [`DDL_TASK_TIMEOUT`]; a replica that is down replays
    /// it on return. A caller bounding the call keeps its bound above that,
    /// so a statement that finished is never taken for one that did not.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the statement is not this connection's to run, or
    /// if the warehouse refuses it.
    pub async fn execute_ddl(&self, ddl: &Ddl) -> Result<(), Error> {
        let sql = self.ddl_sql(ddl)?;
        tracing::debug!(sql = %sql, "running data-layer DDL");
        let mut query = self.query(&sql);
        for (name, value) in coordination::settings(self.config.topology) {
            query = query.with_setting(name, value);
        }
        query.execute().await?;
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

    /// Inside a `Replicated` database ClickHouse rejects `ON CLUSTER` on table
    /// DDL; the cluster is named once, on `CREATE DATABASE`, elsewhere.
    #[test]
    fn a_statement_carries_no_cluster_clause_on_either_topology() -> Result<(), Error> {
        for topology in [Topology::Standalone, Topology::new(true, "insight_cluster")] {
            let client =
                Client::new(Config::new("http://ch:8123", "identity").with_topology(topology));

            let sql = client.ddl_sql(&create_in("identity"))?;

            assert!(!sql.contains("ON CLUSTER"), "{sql}");
        }
        Ok(())
    }

    /// A `Replicated` database replays the DDL but replicates no rows, so the
    /// client's topology is what puts the prefix on the engine.
    #[test]
    fn the_connection_s_topology_decides_the_engine_family() -> Result<(), Error> {
        let replicated = Client::new(
            Config::new("http://ch:8123", "identity")
                .with_topology(Topology::new(true, "insight_cluster")),
        )
        .ddl_sql(&create_in("identity"))?;
        let standalone = Client::new(Config::new("http://ch:8123", "identity"))
            .ddl_sql(&create_in("identity"))?;

        assert!(
            replicated.contains("ENGINE = ReplicatedMergeTree ORDER BY"),
            "{replicated}"
        );
        assert!(
            standalone.contains("ENGINE = MergeTree ORDER BY"),
            "{standalone}"
        );
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

    fn against(mock: &clickhouse::test::Mock, topology: Topology) -> Client {
        Client::new(Config::new(mock.url(), "identity").with_topology(topology))
    }

    fn engine(mock: &clickhouse::test::Mock, engine: &str) {
        mock.add(clickhouse::test::handlers::provide(vec![engine.to_owned()]));
    }

    #[tokio::test]
    async fn a_database_whose_engine_agrees_with_the_topology_passes() -> Result<(), Error> {
        let mock = clickhouse::test::Mock::new();

        engine(&mock, "Atomic");
        against(&mock, Topology::Standalone)
            .assert_database_topology("identity")
            .await?;
        engine(&mock, "Replicated");
        against(&mock, Topology::Replicated)
            .assert_database_topology("identity")
            .await?;
        Ok(())
    }

    /// `create-databases.sh` is the one creation site, so the remedy names it.
    #[tokio::test]
    async fn a_missing_database_is_reported_with_its_creation_site() {
        let mock = clickhouse::test::Mock::new();
        mock.add(clickhouse::test::handlers::provide(Vec::<String>::new()));

        let refused = against(&mock, Topology::Standalone)
            .assert_database_topology("identity")
            .await;

        let Err(Error::Topology(remedy)) = refused else {
            panic!("a missing database is refused: {refused:?}");
        };
        assert!(remedy.contains("create-databases.sh"), "{remedy}");
    }

    #[tokio::test]
    async fn an_engine_that_contradicts_the_topology_is_refused_in_both_directions() {
        let cases = [
            (
                Topology::Standalone,
                "Replicated",
                "clickhouse_cluster_mode is off",
            ),
            (
                Topology::Replicated,
                "Atomic",
                "clickhouse_cluster_mode is on",
            ),
        ];

        for (topology, found, expected) in cases {
            let mock = clickhouse::test::Mock::new();
            engine(&mock, found);

            let refused = against(&mock, topology)
                .assert_database_topology("identity")
                .await;

            let Err(Error::Topology(remedy)) = refused else {
                panic!("should refuse {found} under {topology:?}: {refused:?}");
            };
            assert!(remedy.contains(expected), "{topology:?}/{found}: {remedy}");
            assert!(remedy.contains('\n'), "a remedy is a second line: {remedy}");
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

    #[derive(Debug, serde::Serialize, serde::Deserialize, clickhouse::Row)]
    struct IdRow {
        id: u64,
    }

    /// Every call below, against a warehouse that records what it was asked.
    async fn recorded(
        topology: Topology,
    ) -> Result<Vec<test_support::Received>, Box<dyn std::error::Error>> {
        let capture = test_support::Capture::start().await?;
        let client = Client::new(Config::new(capture.url(), "identity").with_topology(topology));

        client
            .query("SELECT id FROM t")
            .fetch_all::<IdRow>()
            .await?;
        let mut plain = client.inner().insert::<IdRow>("t").await?;
        plain.write(&IdRow { id: 1 }).await?;
        plain.end().await?;
        client
            .consistent_query("SELECT id FROM t")
            .fetch_all::<IdRow>()
            .await?;
        let mut confirmed = client.consistent_insert::<IdRow>("t").await?;
        confirmed.write(&IdRow { id: 1 }).await?;
        confirmed.end().await?;
        client
            .execute_ddl(&Ddl::drop_table("identity", "t"))
            .await?;

        Ok(capture.received())
    }

    const QUORUM_PAIR: [&str; 3] = [
        "insert_quorum",
        "insert_quorum_parallel",
        "select_sequential_consistency",
    ];
    const DDL_WAIT: [&str; 2] = [
        "distributed_ddl_output_mode",
        "distributed_ddl_task_timeout",
    ];

    fn carries(received: &test_support::Received, names: &[&str]) -> bool {
        names.iter().any(|name| received.pair(name).is_some())
    }

    /// Only the `consistent_*` pair opts a call into the quorum settings, and
    /// only a statement waits on the replicas: a plain read or write carries
    /// neither, whatever the topology.
    #[tokio::test]
    async fn only_the_consistent_pair_and_a_statement_carry_cluster_settings()
    -> Result<(), Box<dyn std::error::Error>> {
        let received = recorded(Topology::new(true, "insight_cluster")).await?;
        let Ok([plain_read, plain_write, read, write, ddl]) =
            <[test_support::Received; 5]>::try_from(received)
        else {
            panic!("five requests are sent, one per call");
        };

        assert!(!carries(&plain_read, &QUORUM_PAIR), "{plain_read:?}");
        assert!(!carries(&plain_write, &QUORUM_PAIR), "{plain_write:?}");
        assert_eq!(read.pair("select_sequential_consistency"), Some("1"));
        assert_eq!(write.pair("insert_quorum"), Some("auto"));
        assert_eq!(write.pair("insert_quorum_parallel"), Some("0"));
        assert_eq!(
            ddl.pair("distributed_ddl_output_mode"),
            Some("throw_only_active")
        );
        assert_eq!(
            ddl.pair("distributed_ddl_task_timeout"),
            Some(DDL_TASK_TIMEOUT.as_secs().to_string().as_str())
        );
        for other in [&plain_read, &plain_write, &read, &write] {
            assert!(!carries(other, &DDL_WAIT), "{other:?}");
        }

        Ok(())
    }

    #[tokio::test]
    async fn a_single_node_is_sent_none_of_them() -> Result<(), Box<dyn std::error::Error>> {
        let received = recorded(Topology::Standalone).await?;

        assert_eq!(received.len(), 5, "one request per call: {received:?}");
        for received in received {
            assert!(!carries(&received, &QUORUM_PAIR), "{received:?}");
            assert!(!carries(&received, &DDL_WAIT), "{received:?}");
        }

        Ok(())
    }
}
