//! ClickHouse writer for `identity.identity_persons` — the persons-log copy
//! the metrics dbt builds resolve against.
//!
//! The live table is NOT this service's to create: dbt's
//! `create_identity_persons` on-run-start hook is its one creator. What lives
//! here is the runtime lifecycle — a full-snapshot replace with an atomic swap:
//!
//! 1. create a staging table UNIQUE to this run (suffix = UUIDv7) with the
//!    CURRENT schema — concurrent syncs (another replica's worker) can never
//!    write into or drop each other's staging, and the swap below upgrades
//!    the live table's schema for free on the run after a schema change;
//! 2. stream every row into staging (readers keep seeing the old snapshot);
//! 3. count-verify staging against what we sent — a short write MUST NOT be
//!    swapped in;
//! 4. watermark guard: if the live table already carries a `_synced_at`
//!    NEWER than this snapshot's, abort — a swap would regress the table.
//!    This is a BACKSTOP, not the serialization: concurrent runs are
//!    serialized cluster-wide by the persons-sync advisory lock the worker
//!    holds around the whole run (`infra::db::persons_sync_lock`), which is
//!    what makes check→swap safe. The guard still catches anything that
//!    bypasses the worker (a by-hand EXCHANGE, a future lock-free caller);
//! 5. `EXCHANGE TABLES` — atomic, readers never observe an empty/partial
//!    table (requires an Atomic database, ClickHouse's default). An install
//!    whose first sync beats its first dbt run has no second side to exchange
//!    with, and this run's staging is renamed into place instead;
//! 6. drop this run's staging (post-swap it holds the previous snapshot);
//!    stagings orphaned by crashed runs are garbage-collected at the start
//!    of every run once they are an hour old.
//!
//! Any failure before the swap leaves the live table untouched.
//!
//! On a replicated warehouse the verify and the guard are read behind a load
//! balancer, so they can reach a replica that has not merged the parts this
//! run just wrote — see [`QUORUM_INSERT_SETTINGS`].

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use clickhouse::Row;
use insight_clickhouse::{Client, Config, Topology};
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

/// Read-after-write for the verify → guard → swap sequence on a replicated
/// warehouse. Without a quorum the count read can reach a replica that has not
/// merged this run's parts and abort a complete snapshot as short.
/// `insert_quorum_parallel = 0` is what gives [`CONSISTENT_READ_SETTINGS`] its
/// meaning: ClickHouse does not guarantee sequential consistency while quorum
/// inserts run in parallel.
const QUORUM_INSERT_SETTINGS: [(&str, &str); 2] =
    [("insert_quorum", "auto"), ("insert_quorum_parallel", "0")];
/// Pins the reads the swap decides on to the quorum-confirmed parts.
const CONSISTENT_READ_SETTINGS: [(&str, &str); 1] = [("select_sequential_consistency", "1")];

/// Column block of this run's staging table, and so — through the swap — of the
/// live table's next schema. Mirrors the MariaDB `persons` log
/// (`001_persons.sql`, nullability per
/// `009_align_existing_tables_to_conventions.sql`) minus the generated
/// `value_hash`, plus the `_synced_at` watermark (same convention as
/// `identity_inputs`). Keep in sync with `create_identity_persons.sql`, the
/// hook that creates the live table: it is the shape a build meets before the
/// first sync, and the shape a fresh install's `RENAME` has to match.
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

/// Whether anything has created the live table yet — the one fact that decides
/// both the watermark guard and the swap verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// Created, and carrying whatever snapshot was published last.
    Published,
    /// Absent: the dbt hook that creates it has not run on this install yet.
    Absent,
}

/// The statement that makes this run's staging the live table. `EXCHANGE` needs
/// both sides, so an absent target is reached by `RENAME` — publishing the
/// snapshot without declaring a schema the hook does not write itself.
fn swap_sql(staging: &str, target: Target) -> String {
    match target {
        Target::Published => {
            format!("EXCHANGE TABLES {DATABASE}.`{staging}` AND {DATABASE}.{TARGET}")
        }
        Target::Absent => format!("RENAME TABLE {DATABASE}.`{staging}` TO {DATABASE}.{TARGET}"),
    }
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
    pub fn connect(url: &str, user: &str, password: &str, topology: Topology) -> Self {
        let mut config = Config::new(url, DATABASE)
            .with_query_timeout(WRITE_TIMEOUT)
            .with_topology(topology);
        if !user.is_empty() {
            config = config.with_auth(user, password);
        }
        Self::new(Client::new(config))
    }

    async fn execute(&self, sql: &str) -> anyhow::Result<()> {
        self.client.query(sql).execute().await?;
        Ok(())
    }

    /// A read the swap decides on, pinned to the quorum-confirmed parts when
    /// the warehouse replicates.
    fn consistent_query(&self, sql: &str) -> clickhouse::query::Query {
        let mut query = self.client.query(sql);
        if self.client.config().topology.is_replicated() {
            for (name, value) in CONSISTENT_READ_SETTINGS {
                query = query.with_setting(name, value);
            }
        }
        query
    }

    /// Whether the live table exists. Read fresh for every swap: the hook that
    /// creates it runs on its own schedule, not this service's.
    async fn target_state(&self) -> anyhow::Result<Target> {
        let created: u64 = self
            .client
            .query("SELECT count() FROM system.tables WHERE database = ? AND name = ?")
            .bind(DATABASE)
            .bind(TARGET)
            .fetch_one()
            .await?;
        Ok(if created == 0 {
            Target::Absent
        } else {
            Target::Published
        })
    }

    /// Refuse to publish behind a snapshot already on the table — a swap would
    /// regress it. Empty table → epoch 0, always passes. Equal stamps pass:
    /// re-publishing an identical-instant snapshot is harmless, and the replica
    /// clocks feeding `_synced_at` are the service's own.
    async fn guard_watermark(&self, synced_at: chrono::DateTime<Utc>) -> anyhow::Result<()> {
        let published_ms: i64 = self
            .consistent_query(&format!(
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
        if self.client.config().topology.is_replicated() {
            for (name, value) in QUORUM_INSERT_SETTINGS {
                insert = insert.with_setting(name, value);
            }
        }
        for row in rows {
            insert.write(&to_wire_row(row, synced_at)).await?;
        }
        insert.end().await?;

        // A lost batch must never be swapped in as "the new truth".
        let count: u64 = self
            .consistent_query(&format!("SELECT count() FROM {DATABASE}.`{staging}`"))
            .fetch_one()
            .await?;
        let expected = rows.len() as u64;
        anyhow::ensure!(
            count == expected,
            "staging count mismatch: inserted {expected}, staging holds {count}; \
             aborting swap (live table left untouched)"
        );

        // SAFETY: read as late as possible — a creator racing in between here and
        // the RENAME below fails this run loudly, and the next one exchanges.
        let target = self.target_state().await?;
        match target {
            Target::Published => self.guard_watermark(synced_at).await?,
            Target::Absent => {}
        }

        self.execute(&swap_sql(staging, target)).await?;
        Ok(())
    }

    async fn probe_published_max_id(&self) -> anyhow::Result<Option<u64>> {
        // A missing table/database (fresh install, sync running before the
        // dbt hook that creates it) means "nothing published" — the copy path
        // renames this run's staging into place and surfaces any real
        // connectivity error loudly.
        let probed: Result<Vec<Option<u64>>, _> = self
            .client
            .query(&format!(
                "SELECT if(count() = 0, NULL, max(id)) FROM {DATABASE}.{TARGET}"
            ))
            .fetch_all()
            .await;
        match probed {
            Ok(rows) => Ok(rows.into_iter().next().flatten()),
            Err(e) => {
                tracing::warn!(error = %e, "identity_persons probe failed; publishing");
                Ok(None)
            }
        }
    }
}

#[async_trait]
impl IdentityPersonsWriter for ClickHouseIdentityPersonsWriter {
    async fn published_max_id(&self) -> anyhow::Result<Option<u64>> {
        self.probe_published_max_id().await
    }

    async fn replace(&self, rows: &[PersonsLogRow], synced_at: DateTime) -> anyhow::Result<()> {
        let synced_at = synced_at.and_utc();
        // Unique per run: concurrent syncs never touch each other's staging.
        let staging = format!("{STAGING_PREFIX}{}", Uuid::now_v7().simple());

        // The database normally pre-exists (the deploy's create-databases.sh),
        // but a fresh environment may not have run it yet — idempotent and cheap.
        self.execute(&format!("CREATE DATABASE IF NOT EXISTS {DATABASE}"))
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

    #[test]
    fn a_published_table_is_swapped_for_atomically() {
        let sql = swap_sql("identity_persons_staging_abc", Target::Published);

        assert_eq!(
            sql,
            "EXCHANGE TABLES identity.`identity_persons_staging_abc` AND identity.identity_persons"
        );
    }

    #[test]
    fn a_table_no_creator_has_made_yet_is_renamed_into_place() {
        let sql = swap_sql("identity_persons_staging_abc", Target::Absent);

        assert_eq!(
            sql,
            "RENAME TABLE identity.`identity_persons_staging_abc` TO identity.identity_persons"
        );
    }

    #[test]
    fn publishing_never_creates_the_table_dbt_owns() {
        for target in [Target::Published, Target::Absent] {
            let sql = swap_sql("identity_persons_staging_abc", target);

            assert!(
                !sql.contains("CREATE"),
                "the swap must not become a second creator: {sql}"
            );
        }
    }
}
