//! ClickHouse writer for `identity.identity_persons` — the persons-log copy
//! the metrics dbt builds resolve against.
//!
//! Full-snapshot replace with an atomic swap:
//!
//! 1. `CREATE TABLE IF NOT EXISTS` the target (first run / dbt-hook parity);
//! 2. create a staging table UNIQUE to this run (suffix = UUIDv7) with the
//!    CURRENT schema — concurrent syncs (another replica's worker) can never
//!    write into or drop each other's staging, and the swap below upgrades
//!    the live table's schema for free on the run after a schema change;
//! 3. stream every row into staging (readers keep seeing the old snapshot);
//! 4. count-verify staging against what we sent — a short write MUST NOT be
//!    swapped in;
//! 5. watermark guard: if the live table already carries a `_synced_at`
//!    NEWER than this snapshot's, abort — a swap would regress the table.
//!    This is a BACKSTOP, not the serialization: concurrent runs are
//!    serialized cluster-wide by the persons-sync advisory lock the worker
//!    holds around the whole run (`infra::db::persons_sync_lock`), which is
//!    what makes check→swap safe. The guard still catches anything that
//!    bypasses the worker (a by-hand EXCHANGE, a future lock-free caller);
//! 6. `EXCHANGE TABLES` — atomic, readers never observe an empty/partial
//!    table (requires an Atomic database, ClickHouse's default);
//! 7. drop this run's staging (post-swap it holds the previous snapshot);
//!    stagings orphaned by crashed runs are garbage-collected at the start
//!    of every run once they are an hour old.
//!
//! Any failure before the swap leaves the live table untouched.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use clickhouse::Row;
use insight_clickhouse::{Client, Config};
use sea_orm::prelude::DateTime;
use serde::Serialize;
use uuid::Uuid;

use crate::domain::sync_service::{IdentityPersonsWriter, PersonsLogRow};

/// The whole snapshot (DDL + insert + verify) rides one client; generous bound,
/// the sync operation as a whole is separately bounded by the worker.
const WRITE_TIMEOUT: Duration = Duration::from_mins(5);

const DATABASE: &str = "identity";
const TARGET: &str = "identity_persons";
/// Per-run staging tables are `identity_persons_staging_<uuidv7-simple>`.
const STAGING_PREFIX: &str = "identity_persons_staging_";
/// How old an orphaned staging table must be before the GC drops it — old
/// enough that no live run (bounded well under this by `SYNC_TIMEOUT`) can
/// still be writing to it.
const STAGING_GC_AGE_SECONDS: u32 = 3600;

/// Column block shared by the target and staging DDL. Mirrors the MariaDB
/// `persons` log (`001_persons.sql`, nullability per
/// `009_align_existing_tables_to_conventions.sql`) minus the generated
/// `value_hash`, plus the `_synced_at` watermark (same convention as
/// `identity_inputs`). Keep in sync with the dbt on-run-start hook that
/// creates the empty table for builds that run before the first sync.
const COLUMNS_DDL: &str = r"
    id                  UInt64,
    value_type          String,
    insight_source_type String,
    insight_source_id   UUID,
    insight_tenant_id   UUID,
    value_id            Nullable(String),
    value_full_text     Nullable(String),
    value               Nullable(String),
    value_effective     Nullable(String),
    person_id           UUID,
    author_person_id    UUID,
    reason              Nullable(String),
    created_at          DateTime64(6, 'UTC'),
    _synced_at          DateTime64(3, 'UTC')
";

/// Wire row for the `RowBinary` insert. Field order and names must match the
/// DDL above — the clickhouse client sends `INSERT INTO … (field names)`.
/// The watermark field is serde-renamed to `_synced_at` (a Rust field can't
/// comfortably live with the underscore prefix under clippy).
#[derive(Debug, Row, Serialize)]
struct WireRow {
    id: u64,
    value_type: String,
    insight_source_type: String,
    #[serde(with = "clickhouse::serde::uuid")]
    insight_source_id: Uuid,
    #[serde(with = "clickhouse::serde::uuid")]
    insight_tenant_id: Uuid,
    value_id: Option<String>,
    value_full_text: Option<String>,
    value: Option<String>,
    value_effective: Option<String>,
    #[serde(with = "clickhouse::serde::uuid")]
    person_id: Uuid,
    #[serde(with = "clickhouse::serde::uuid")]
    author_person_id: Uuid,
    reason: Option<String>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::micros")]
    created_at: chrono::DateTime<Utc>,
    #[serde(
        rename = "_synced_at",
        with = "clickhouse::serde::chrono::datetime64::millis"
    )]
    synced_at: chrono::DateTime<Utc>,
}

/// [`IdentityPersonsWriter`] over the shared `insight-clickhouse` client.
pub struct ClickHouseIdentityPersonsWriter {
    client: Client,
}

impl ClickHouseIdentityPersonsWriter {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Build a writer from connection settings (empty user → no auth). The
    /// client database is pinned to `identity` regardless of the configured
    /// read database — the table's home is fixed by contract.
    #[must_use]
    pub fn connect(url: &str, user: &str, password: &str) -> Self {
        let mut config = Config::new(url, DATABASE).with_query_timeout(WRITE_TIMEOUT);
        if !user.is_empty() {
            config = config.with_auth(user, password);
        }
        Self::new(Client::new(config))
    }

    async fn execute(&self, sql: &str) -> anyhow::Result<()> {
        self.client.query(sql).execute().await?;
        Ok(())
    }

    /// Drop staging tables orphaned by crashed runs. Only tables older than
    /// [`STAGING_GC_AGE_SECONDS`] — a younger one may belong to a live
    /// concurrent run on another replica. Best-effort: GC failures must not
    /// fail the sync.
    async fn drop_stale_stagings(&self) {
        let stale: Result<Vec<String>, _> = self
            .client
            .query(
                "SELECT name FROM system.tables \
                 WHERE database = ? AND name LIKE ? \
                   AND metadata_modification_time < now() - INTERVAL ? SECOND",
            )
            .bind(DATABASE)
            .bind(format!("{STAGING_PREFIX}%"))
            .bind(STAGING_GC_AGE_SECONDS)
            .fetch_all()
            .await;
        let stale = match stale {
            Ok(names) => names,
            Err(e) => {
                tracing::warn!(error = %e, "persons-sync: staging GC listing failed (skipped)");
                return;
            }
        };
        for name in stale {
            // Defense in depth: only names matching exactly what this code
            // mints (prefix + 32 lowercase hex chars of a simple-format UUID)
            // ever reach the identifier-interpolated DROP, even if the LIKE
            // above were somehow loosened.
            let Some(suffix) = name.strip_prefix(STAGING_PREFIX) else {
                continue;
            };
            if suffix.len() != 32 || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            match self
                .execute(&format!("DROP TABLE IF EXISTS {DATABASE}.`{name}`"))
                .await
            {
                Ok(()) => tracing::info!(table = %name, "persons-sync: dropped orphaned staging"),
                Err(e) => {
                    tracing::warn!(error = %e, table = %name, "persons-sync: staging GC drop failed");
                }
            }
        }
    }

    /// Insert + verify + guard + swap against `staging`. Split out so
    /// [`replace`](IdentityPersonsWriter::replace) can unconditionally drop this
    /// run's staging afterwards, on success and failure alike.
    async fn fill_and_swap(
        &self,
        staging: &str,
        rows: &[PersonsLogRow],
        synced_at: chrono::DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let mut insert = self.client.inner().insert::<WireRow>(staging).await?;
        for row in rows {
            insert.write(&to_wire_row(row, synced_at)).await?;
        }
        insert.end().await?;

        // A lost batch must never be swapped in as "the new truth".
        let count: u64 = self
            .client
            .query(&format!("SELECT count() FROM {DATABASE}.`{staging}`"))
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
        // harmless, and the replica clocks feeding `_synced_at` are the
        // service's own.
        let published_ms: i64 = self
            .client
            .query(&format!(
                "SELECT toUnixTimestamp64Milli(max(_synced_at)) FROM {DATABASE}.{TARGET}"
            ))
            .fetch_one()
            .await?;
        anyhow::ensure!(
            published_ms <= synced_at.timestamp_millis(),
            "a newer snapshot (_synced_at={published_ms}ms) is already published; \
             discarding this run's older snapshot ({}ms)",
            synced_at.timestamp_millis()
        );

        self.execute(&format!(
            "EXCHANGE TABLES {DATABASE}.`{staging}` AND {DATABASE}.{TARGET}"
        ))
        .await?;
        Ok(())
    }
}

#[async_trait]
impl IdentityPersonsWriter for ClickHouseIdentityPersonsWriter {
    async fn replace(&self, rows: &[PersonsLogRow], synced_at: DateTime) -> anyhow::Result<()> {
        let synced_at = synced_at.and_utc();
        // Unique per run: concurrent syncs never touch each other's staging.
        let staging = format!("{STAGING_PREFIX}{}", Uuid::now_v7().simple());

        // The database normally pre-exists (init-identity migration), but a
        // fresh environment may not have run it yet — idempotent and cheap.
        self.execute(&format!("CREATE DATABASE IF NOT EXISTS {DATABASE}"))
            .await?;
        // Target first: EXCHANGE requires both sides to exist, and the very
        // first sync runs against a cluster that may only have the database.
        self.execute(&format!(
            "CREATE TABLE IF NOT EXISTS {DATABASE}.{TARGET} ({COLUMNS_DDL}) \
             ENGINE = MergeTree ORDER BY id"
        ))
        .await?;
        self.drop_stale_stagings().await;

        self.execute(&format!(
            "CREATE TABLE {DATABASE}.`{staging}` ({COLUMNS_DDL}) \
             ENGINE = MergeTree ORDER BY id"
        ))
        .await?;

        let result = self.fill_and_swap(&staging, rows, synced_at).await;

        // Unconditional cleanup of THIS run's staging: after a successful swap
        // it holds the previous snapshot; after a failure, the partial write.
        // Best-effort — an orphan is reclaimed by the next run's GC.
        if let Err(e) = self
            .execute(&format!("DROP TABLE IF EXISTS {DATABASE}.`{staging}`"))
            .await
        {
            tracing::warn!(error = %e, table = %staging, "persons-sync: dropping own staging failed");
        }
        result
    }
}

fn to_wire_row(r: &PersonsLogRow, synced_at: chrono::DateTime<Utc>) -> WireRow {
    WireRow {
        id: r.id,
        value_type: r.value_type.clone(),
        insight_source_type: r.insight_source_type.clone(),
        insight_source_id: r.insight_source_id,
        insight_tenant_id: r.insight_tenant_id,
        value_id: r.value_id.clone(),
        value_full_text: r.value_full_text.clone(),
        value: r.value.clone(),
        value_effective: r.value_effective.clone(),
        person_id: r.person_id,
        author_person_id: r.author_person_id,
        reason: r.reason.clone(),
        // MariaDB `TIMESTAMP(6)` comes back naive; the pool session runs in
        // UTC, so re-attaching Utc is a re-labeling, not a conversion.
        created_at: r.created_at.and_utc(),
        synced_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::sync_service::PersonsLogRow;

    #[test]
    fn maps_log_row_preserving_micros_and_nulls() -> anyhow::Result<()> {
        let created =
            DateTime::parse_from_str("2026-07-29 12:34:56.123456", "%Y-%m-%d %H:%M:%S%.f")?;
        let row = PersonsLogRow {
            id: 9,
            value_type: "email".to_owned(),
            insight_source_type: "bamboohr".to_owned(),
            insight_source_id: Uuid::from_u128(1),
            insight_tenant_id: Uuid::from_u128(2),
            value_id: Some("a@x.com".to_owned()),
            value_full_text: None,
            value: None,
            value_effective: Some("a@x.com".to_owned()),
            person_id: Uuid::from_u128(3),
            author_person_id: Uuid::from_u128(4),
            reason: None,
            created_at: created,
        };
        let synced =
            DateTime::parse_from_str("2026-07-29 13:00:00", "%Y-%m-%d %H:%M:%S")?.and_utc();

        let wire = to_wire_row(&row, synced);

        assert_eq!(wire.id, 9);
        assert_eq!(wire.created_at.timestamp_subsec_micros(), 123_456);
        assert_eq!(wire.synced_at, synced);
        // A NULL reason stays NULL — the copy is verbatim, not normalized.
        assert_eq!(wire.reason, None);
        Ok(())
    }
}
