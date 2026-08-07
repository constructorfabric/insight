//! Full-snapshot ClickHouse publisher with an atomic swap, shared by every
//! relation this service publishes.
//!
//! 1. `CREATE TABLE IF NOT EXISTS` the target (first run / dbt-hook parity);
//! 2. create a staging table UNIQUE to this run (suffix = UUIDv7) with the
//!    CURRENT schema — concurrent runs (another replica's worker) can never
//!    write into or drop each other's staging, and the swap below upgrades
//!    the live table's schema for free on the run after a schema change;
//! 3. stream every row into staging (readers keep seeing the old snapshot);
//! 4. count-verify staging against what we sent — a short write MUST NOT be
//!    swapped in;
//! 5. watermark guard: if the live table already carries a watermark NEWER
//!    than this snapshot's, abort — a swap would regress the table. This is
//!    a BACKSTOP, not the serialization: concurrent runs are serialized
//!    cluster-wide by the caller's advisory lock, which is what makes
//!    check→swap safe. The guard still catches anything that bypasses the
//!    worker (a by-hand EXCHANGE, a future lock-free caller);
//! 6. `EXCHANGE TABLES` — atomic, readers never observe an empty/partial
//!    table (requires an Atomic database, ClickHouse's default);
//! 7. drop this run's staging (post-swap it holds the previous snapshot);
//!    stagings orphaned by crashed runs are garbage-collected at the start
//!    of every run once they are an hour old.
//!
//! Any failure before the swap leaves the live table untouched.

use std::time::Duration;

use chrono::Utc;
use clickhouse::Row;
use insight_clickhouse::{Client, Config};
use serde::Serialize;
use uuid::Uuid;

/// How old an orphaned staging table must be before the GC drops it — old
/// enough that no live run (bounded well under this by its caller's timeout)
/// can still be writing to it.
const STAGING_GC_AGE_SECONDS: u32 = 3600;

/// Length of the UUID-simple suffix this module mints for staging names.
const STAGING_SUFFIX_LEN: usize = 32;

/// Everything that distinguishes one published relation from another.
///
/// INVARIANT: no `staging_prefix` may be a prefix of another spec's — the GC
/// lists by `LIKE '<prefix>%'`, so overlapping prefixes would let one
/// relation's GC drop another's live staging.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotSpec {
    pub database: &'static str,
    pub target: &'static str,
    pub staging_prefix: &'static str,
    pub columns_ddl: &'static str,
    pub order_by: &'static str,
    pub watermark_column: &'static str,
    /// Prefix for this relation's operational log lines.
    pub log_label: &'static str,
}

/// Publishes full snapshots of one relation.
pub struct SnapshotWriter {
    client: Client,
    spec: SnapshotSpec,
}

impl SnapshotWriter {
    #[must_use]
    pub fn new(client: Client, spec: SnapshotSpec) -> Self {
        Self { client, spec }
    }

    /// Build a writer from connection settings (empty user → no auth). The
    /// client database is pinned to the spec's — the table's home is fixed by
    /// contract, regardless of the configured read database.
    #[must_use]
    pub fn connect(
        url: &str,
        user: &str,
        password: &str,
        spec: SnapshotSpec,
        write_timeout: Duration,
    ) -> Self {
        let mut config = Config::new(url, spec.database).with_query_timeout(write_timeout);
        if !user.is_empty() {
            config = config.with_auth(user, password);
        }
        Self::new(Client::new(config), spec)
    }

    /// Rows currently published in the live target — the count half of a
    /// caller's "is the published snapshot still the one I journalled" check.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails. A missing target is an error, not
    /// zero: callers distinguish "nothing published" from "cannot tell".
    pub async fn published_row_count(&self) -> anyhow::Result<u64> {
        let SnapshotSpec {
            database, target, ..
        } = self.spec;
        let count: u64 = self
            .client
            .query(&format!("SELECT count() FROM {database}.{target}"))
            .fetch_one()
            .await?;
        Ok(count)
    }

    async fn execute(&self, sql: &str) -> anyhow::Result<()> {
        self.client.query(sql).execute().await?;
        Ok(())
    }

    /// `CREATE DATABASE` cannot run on a client pinned to that database —
    /// ClickHouse resolves the request's database before executing and
    /// rejects the statement outright — so the bootstrap DDL goes through
    /// `default`, which always exists.
    async fn ensure_database(&self) -> anyhow::Result<()> {
        let database = self.spec.database;
        self.client
            .inner()
            .clone()
            .with_database("default")
            .query(&format!("CREATE DATABASE IF NOT EXISTS {database}"))
            .execute()
            .await?;
        Ok(())
    }

    /// Drop staging tables orphaned by crashed runs. Only tables older than
    /// [`STAGING_GC_AGE_SECONDS`] — a younger one may belong to a live
    /// concurrent run on another replica. Best-effort: GC failures must not
    /// fail the publish.
    async fn drop_stale_stagings(&self) {
        let SnapshotSpec {
            database,
            staging_prefix,
            log_label,
            ..
        } = self.spec;
        let stale: Result<Vec<String>, _> = self
            .client
            .query(
                "SELECT name FROM system.tables \
                 WHERE database = ? AND name LIKE ? \
                   AND metadata_modification_time < now() - INTERVAL ? SECOND",
            )
            .bind(database)
            .bind(format!("{staging_prefix}%"))
            .bind(STAGING_GC_AGE_SECONDS)
            .fetch_all()
            .await;
        let stale = match stale {
            Ok(names) => names,
            Err(e) => {
                tracing::warn!(error = %e, "{log_label}: staging GC listing failed (skipped)");
                return;
            }
        };
        for name in stale {
            // Defense in depth: only names matching exactly what this code
            // mints (prefix + 32 lowercase hex chars of a simple-format UUID)
            // ever reach the identifier-interpolated DROP, even if the LIKE
            // above were somehow loosened.
            let Some(suffix) = name.strip_prefix(staging_prefix) else {
                continue;
            };
            if suffix.len() != STAGING_SUFFIX_LEN || !suffix.bytes().all(|b| b.is_ascii_hexdigit())
            {
                continue;
            }
            match self
                .execute(&format!("DROP TABLE IF EXISTS {database}.`{name}`"))
                .await
            {
                Ok(()) => tracing::info!(table = %name, "{log_label}: dropped orphaned staging"),
                Err(e) => {
                    tracing::warn!(error = %e, table = %name, "{log_label}: staging GC drop failed");
                }
            }
        }
    }

    /// Insert + verify + guard + swap against `staging`. Split out so
    /// [`replace`](Self::replace) can unconditionally drop this run's staging
    /// afterwards, on success and failure alike.
    async fn fill_and_swap<R>(
        &self,
        staging: &str,
        rows: &[R],
        watermark: chrono::DateTime<Utc>,
    ) -> anyhow::Result<()>
    where
        R: Row + Serialize + Send + Sync,
        for<'a> R: Row<Value<'a> = R>,
    {
        let SnapshotSpec {
            database,
            target,
            watermark_column,
            ..
        } = self.spec;

        let mut insert = self.client.inner().insert::<R>(staging).await?;
        for row in rows {
            insert.write(row).await?;
        }
        insert.end().await?;

        // A lost batch must never be swapped in as "the new truth".
        let count: u64 = self
            .client
            .query(&format!("SELECT count() FROM {database}.`{staging}`"))
            .fetch_one()
            .await?;
        let expected = rows.len() as u64;
        anyhow::ensure!(
            count == expected,
            "staging count mismatch: inserted {expected}, staging holds {count}; \
             aborting swap (live table left untouched)"
        );

        // Watermark guard (empty table → epoch 0, always passes). Equal
        // stamps pass: re-publishing an identical-instant snapshot is
        // harmless, and the replica clocks feeding the watermark are the
        // service's own.
        let published_ms: i64 = self
            .client
            .query(&format!(
                "SELECT toUnixTimestamp64Milli(max({watermark_column})) FROM {database}.{target}"
            ))
            .fetch_one()
            .await?;
        anyhow::ensure!(
            published_ms <= watermark.timestamp_millis(),
            "a newer snapshot ({watermark_column}={published_ms}ms) is already published; \
             discarding this run's older snapshot ({}ms)",
            watermark.timestamp_millis()
        );

        self.execute(&format!(
            "EXCHANGE TABLES {database}.`{staging}` AND {database}.{target}"
        ))
        .await?;
        Ok(())
    }

    /// Replace the live relation with `rows`, atomically.
    ///
    /// # Errors
    ///
    /// Returns an error if any DDL, the insert, the count verification, the
    /// watermark guard, or the swap fails. The live table is untouched unless
    /// the swap itself succeeded.
    pub async fn replace<R>(
        &self,
        rows: &[R],
        watermark: chrono::DateTime<Utc>,
    ) -> anyhow::Result<()>
    where
        R: Row + Serialize + Send + Sync,
        for<'a> R: Row<Value<'a> = R>,
    {
        let SnapshotSpec {
            database,
            target,
            staging_prefix,
            columns_ddl,
            order_by,
            log_label,
            ..
        } = self.spec;

        // Unique per run: concurrent runs never touch each other's staging.
        let staging = format!("{staging_prefix}{}", Uuid::now_v7().simple());

        // The database normally pre-exists (init-identity migration), but a
        // fresh environment may not have run it yet — idempotent and cheap.
        self.ensure_database().await?;
        // Target first: EXCHANGE requires both sides to exist, and the very
        // first run may execute against a cluster that only has the database.
        self.execute(&format!(
            "CREATE TABLE IF NOT EXISTS {database}.{target} ({columns_ddl}) \
             ENGINE = MergeTree ORDER BY {order_by}"
        ))
        .await?;
        self.drop_stale_stagings().await;

        self.execute(&format!(
            "CREATE TABLE {database}.`{staging}` ({columns_ddl}) \
             ENGINE = MergeTree ORDER BY {order_by}"
        ))
        .await?;

        let result = self.fill_and_swap(&staging, rows, watermark).await;

        // Unconditional cleanup of THIS run's staging: after a successful swap
        // it holds the previous snapshot; after a failure, the partial write.
        // Best-effort — an orphan is reclaimed by the next run's GC.
        if let Err(e) = self
            .execute(&format!("DROP TABLE IF EXISTS {database}.`{staging}`"))
            .await
        {
            tracing::warn!(error = %e, table = %staging, "{log_label}: dropping own staging failed");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every spec in the binary, so the prefix invariant is checked centrally
    /// rather than trusted per call site.
    fn all_specs() -> Vec<SnapshotSpec> {
        vec![
            crate::infra::identity_persons::SPEC,
            crate::infra::policy_snapshot::SPEC,
        ]
    }

    #[test]
    fn no_staging_prefix_is_a_prefix_of_another() {
        let specs = all_specs();
        for outer in &specs {
            for inner in &specs {
                if outer.target == inner.target {
                    continue;
                }
                assert!(
                    !inner.staging_prefix.starts_with(outer.staging_prefix),
                    "staging prefix {:?} would be GC'd by {:?}'s sweep",
                    inner.staging_prefix,
                    outer.staging_prefix
                );
            }
        }
    }

    #[test]
    fn every_spec_targets_a_distinct_relation() {
        let specs = all_specs();
        for (i, outer) in specs.iter().enumerate() {
            for inner in &specs[i + 1..] {
                assert_ne!(
                    (outer.database, outer.target),
                    (inner.database, inner.target),
                    "two specs publish the same relation"
                );
            }
        }
    }
}
