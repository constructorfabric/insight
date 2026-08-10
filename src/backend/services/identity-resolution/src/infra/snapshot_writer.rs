use std::time::Duration;

use chrono::Utc;
use clickhouse::Row;
use insight_clickhouse::{Client, Config};
use serde::Serialize;
use uuid::Uuid;

const STAGING_GC_AGE_SECONDS: u32 = 3600;
const STAGING_SUFFIX_LEN: usize = 32;

// INVARIANT: no `staging_prefix` may be a prefix of another spec's — the GC lists by
// `LIKE '<prefix>%'`, so an overlap lets one relation's sweep drop another's live staging.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotSpec {
    pub database: &'static str,
    pub target: &'static str,
    pub staging_prefix: &'static str,
    pub columns_ddl: &'static str,
    pub order_by: &'static str,
    pub watermark_column: &'static str,
    pub log_label: &'static str,
}

pub struct SnapshotWriter {
    client: Client,
    spec: SnapshotSpec,
}

impl SnapshotWriter {
    #[must_use]
    pub fn new(client: Client, spec: SnapshotSpec) -> Self {
        Self { client, spec }
    }

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

    async fn ensure_database(&self) -> anyhow::Result<()> {
        let database = self.spec.database;
        // WORKAROUND: ClickHouse resolves a request's database before executing it, so
        // `CREATE DATABASE` on a client pinned to that database is rejected outright.
        self.client
            .inner()
            .clone()
            .with_database("default")
            .query(&format!("CREATE DATABASE IF NOT EXISTS {database}"))
            .execute()
            .await?;
        Ok(())
    }

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

        // INVARIANT: this guard is a backstop, not the serialization — concurrent runs are
        // serialized by the caller's advisory lock, which is what makes check-then-swap safe.
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

        let staging = format!("{staging_prefix}{}", Uuid::now_v7().simple());

        self.ensure_database().await?;
        // EXCHANGE needs both sides to exist, so the target is created before the staging.
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
