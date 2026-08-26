//! Persons sync — copy the MariaDB `persons` observation log into ClickHouse
//! (`identity.identity_persons`) so the metrics pipeline (dbt gold builds) can
//! resolve `email -> person_id` next to the observation tables it already
//! reads. Iteration 1 of the metrics `person_id` rework: the ClickHouse copy is
//! a disposable full snapshot — MariaDB stays the only source of truth, and
//! every run replaces the whole table (self-healing, no incremental state).
//!
//! Shaped like [`run_seed`]: pure orchestration over two ports so the flow is
//! unit-testable without either database.
//!
//! [`run_seed`]: crate::domain::seed_service::run_seed

use async_trait::async_trait;
use sea_orm::prelude::DateTime;
use serde::Serialize;
use uuid::Uuid;

/// One row of the `persons` observation log, copied VERBATIM — including
/// nullability (`reason` is nullable per migration 009; NULL and `''` stay
/// distinct in `identity_persons`). Matches the MariaDB schema column-for-column
/// except `value_hash` (a generated convenience column, cheap to recompute in
/// ClickHouse if ever needed).
#[derive(Debug, Clone)]
pub struct PersonsLogRow {
    pub id: u64,
    pub value_type: String,
    pub insight_source_type: String,
    pub insight_source_id: Uuid,
    pub insight_tenant_id: Uuid,
    pub value_id: Option<String>,
    pub value_full_text: Option<String>,
    pub value: Option<String>,
    pub value_effective: Option<String>,
    pub person_id: Uuid,
    pub author_person_id: Uuid,
    pub reason: Option<String>,
    pub created_at: DateTime,
}

/// Reads the full `persons` log from the identity store.
#[async_trait]
pub trait PersonsLogReader: Send + Sync {
    /// All rows ordered by `id`. Materialized (not streamed) — same trade-off
    /// as the seed's `IdentityInputsReader`: fine at current sizes.
    async fn read_all(&self) -> anyhow::Result<Vec<PersonsLogRow>>;

    /// The highest `id` the log holds RIGHT NOW (`None` for an empty log) —
    /// the cheap staleness probe behind [`run_sync_until_quiescent`].
    async fn latest_id(&self) -> anyhow::Result<Option<u64>>;
}

/// Replaces ClickHouse `identity.identity_persons` with a new snapshot.
#[async_trait]
pub trait IdentityPersonsWriter: Send + Sync {
    /// Load `rows` (stamped with `synced_at`) into a staging table and swap it
    /// in atomically. Must leave the previous snapshot untouched on any failure.
    async fn replace(&self, rows: &[PersonsLogRow], synced_at: DateTime) -> anyhow::Result<()>;

    /// The highest log `id` the LIVE snapshot carries (`None` for an empty or
    /// absent table) — the other half of the change probe: equal to the log's
    /// [`PersonsLogReader::latest_id`] means there is nothing to publish.
    async fn published_max_id(&self) -> anyhow::Result<Option<u64>>;
}

/// What one publish attempt did. `AlreadyCurrent` is the change probe firing:
/// the snapshot already carries the log's highest id, so no copy ran — what
/// makes a 15-minute backstop and a queue of post-correction publishes cost
/// two point queries instead of a full republish each.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SyncOutcome {
    Published(SyncSummary),
    AlreadyCurrent { max_id: Option<u64> },
}

/// What a completed sync reports (stored as the operation's `summary_json`).
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct SyncSummary {
    /// Rows copied into `identity_persons`.
    pub rows: u64,
    /// Highest log `id` in the snapshot — the resolution watermark; `null`
    /// for an empty log.
    pub max_id: Option<u64>,
    /// Latest observation timestamp in the snapshot (ISO-8601), `null` for an
    /// empty log.
    pub max_created_at: Option<String>,
    /// When this snapshot was taken (ISO-8601). Also stamped on every copied
    /// row as `_synced_at`.
    pub synced_at: String,
}

/// Copy the whole log through the two ports. An empty log is a valid snapshot
/// (`identity_persons` is emptied) — deleting every person in MariaDB should not leave
/// stale resolutions behind.
///
/// # Errors
///
/// Propagates reader/writer failures; the writer contract guarantees the
/// previous snapshot survives them.
/// Why a sync did not publish: the guard refused, or the work failed.
pub enum SyncError {
    /// The log was empty and `--force` was not given. Operator-facing message.
    EmptyLog(String),
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for SyncError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed(e)
    }
}

/// The pure guard decision: refuse publishing an EMPTY log unless `force`.
/// An empty read is far more often a misconfigured database / wiped stand than
/// a real "no people", and publishing it atomically erases a populated mirror.
fn empty_log_guard(log_rows: usize, force: bool) -> Result<(), String> {
    if force || log_rows > 0 {
        return Ok(());
    }
    Err(
        "empty-log guard: the persons log has 0 rows — publishing would erase the \
         ClickHouse snapshot (misconfigured database_url? wiped stand?); re-run with \
         --force to publish the empty snapshot deliberately"
            .to_owned(),
    )
}

pub async fn run_sync(
    reader: &dyn PersonsLogReader,
    writer: &dyn IdentityPersonsWriter,
    now: DateTime,
    force: bool,
) -> Result<SyncSummary, SyncError> {
    let rows = reader.read_all().await?;

    // Guarded on the rows about to be PUBLISHED, not on an earlier count(): a
    // seed running between the two (it holds a different lock) could empty the
    // log after a non-zero count and slip the empty snapshot past the guard.
    if let Err(msg) = empty_log_guard(rows.len(), force) {
        return Err(SyncError::EmptyLog(msg));
    }

    writer.replace(&rows, now).await?;

    Ok(SyncSummary {
        rows: rows.len() as u64,
        max_id: rows.iter().map(|r| r.id).max(),
        max_created_at: rows.iter().map(|r| r.created_at).max().map(fmt_iso),
        synced_at: fmt_iso(now),
    })
}

/// Extra copy passes one publish may spend swallowing rows that landed while
/// a pass was copying. Two is deliberate: one pass covers the common race
/// (a correction landing mid-copy), and a log still moving after three full
/// copies is under constant churn — the next publish is imminent by
/// definition, so chasing it here only holds the lock longer.
pub const MAX_QUIESCENCE_PASSES: u32 = 2;

/// Copy the log and repeat while rows landed during the copy. Best-effort
/// with a documented bound, not an absolute guarantee: the pass cap can leave
/// rows for the next publish, and a waiter that timed out on the lock may be
/// ahead of the holder's last probe — in both cases staleness is bounded by
/// the next publish (a correction, a pipeline seed, or the backstop tick),
/// never unbounded.
///
/// `now` is called once per pass: each snapshot carries its own `_synced_at`,
/// and the writer's watermark guard rejects a regressing stamp (equal stamps
/// are accepted — see `identity_persons.rs`).
///
/// # Errors
///
/// Same contract as [`run_sync`]; the first failing pass ends the loop.
pub async fn run_sync_until_quiescent(
    reader: &dyn PersonsLogReader,
    writer: &dyn IdentityPersonsWriter,
    mut now: impl FnMut() -> DateTime + Send,
    force: bool,
) -> Result<SyncOutcome, SyncError> {
    // Change probe: nothing landed since the live snapshot → nothing to copy.
    // `force` skips the probe — it exists to republish deliberately. An empty
    // log (`latest = None`) falls through to the guard, which owns that case.
    if !force {
        let latest = reader.latest_id().await.map_err(SyncError::Failed)?;
        if latest.is_some()
            && latest == writer.published_max_id().await.map_err(SyncError::Failed)?
        {
            return Ok(SyncOutcome::AlreadyCurrent { max_id: latest });
        }
    }
    let mut summary = run_sync(reader, writer, now(), force).await?;
    for _ in 0..MAX_QUIESCENCE_PASSES {
        let latest = reader.latest_id().await.map_err(SyncError::Failed)?;
        if latest <= summary.max_id {
            return Ok(SyncOutcome::Published(summary));
        }
        summary = run_sync(reader, writer, now(), force).await?;
    }
    if reader.latest_id().await.map_err(SyncError::Failed)? > summary.max_id {
        tracing::warn!(
            passes = 1 + MAX_QUIESCENCE_PASSES,
            "persons-sync: log still moving after the pass cap; the next \
             publish carries the remainder"
        );
    }
    Ok(SyncOutcome::Published(summary))
}

/// ISO-8601 with a `T` separator (same rationale as the seed API's `fmt_ts`).
fn fmt_iso(dt: DateTime) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}

#[cfg(test)]
mod tests {
    use tokio::sync::Mutex;

    use super::*;

    fn row(id: u64, created_at: &str) -> anyhow::Result<PersonsLogRow> {
        Ok(PersonsLogRow {
            id,
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
            created_at: parse(created_at)?,
        })
    }

    fn parse(s: &str) -> anyhow::Result<DateTime> {
        Ok(DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")?)
    }

    struct FakeReader(Vec<PersonsLogRow>);
    #[async_trait]
    impl PersonsLogReader for FakeReader {
        async fn read_all(&self) -> anyhow::Result<Vec<PersonsLogRow>> {
            Ok(self.0.clone())
        }
        async fn latest_id(&self) -> anyhow::Result<Option<u64>> {
            Ok(self.0.iter().map(|r| r.id).max())
        }
    }

    /// Records every replace; async `Mutex` because the trait takes `&self`.
    #[derive(Default)]
    struct FakeWriter {
        replaces: Mutex<Vec<(usize, DateTime)>>,
        published: Mutex<Option<u64>>,
    }
    impl FakeWriter {
        async fn with_published(self, max_id: u64) -> Self {
            *self.published.lock().await = Some(max_id);
            self
        }
        async fn replace_count(&self) -> usize {
            self.replaces.lock().await.len()
        }
        async fn synced_ats(&self) -> Vec<DateTime> {
            self.replaces
                .lock()
                .await
                .iter()
                .map(|(_, at)| *at)
                .collect()
        }
        async fn last(&self) -> Option<(usize, DateTime)> {
            self.replaces.lock().await.last().copied()
        }
    }
    #[async_trait]
    impl IdentityPersonsWriter for FakeWriter {
        async fn replace(&self, rows: &[PersonsLogRow], synced_at: DateTime) -> anyhow::Result<()> {
            self.replaces.lock().await.push((rows.len(), synced_at));
            Ok(())
        }
        async fn published_max_id(&self) -> anyhow::Result<Option<u64>> {
            Ok(*self.published.lock().await)
        }
    }

    /// Versions of the log as successive `read_all` calls will see them —
    /// rows landing between passes are modeled as the queue advancing.
    struct GrowingReader {
        versions: Mutex<Vec<Vec<PersonsLogRow>>>,
    }
    impl GrowingReader {
        fn new(versions: Vec<Vec<PersonsLogRow>>) -> Self {
            Self {
                versions: Mutex::new(versions),
            }
        }
    }
    #[async_trait]
    impl PersonsLogReader for GrowingReader {
        async fn read_all(&self) -> anyhow::Result<Vec<PersonsLogRow>> {
            let mut versions = self.versions.lock().await;
            if versions.len() > 1 {
                Ok(versions.remove(0))
            } else {
                Ok(versions[0].clone())
            }
        }
        async fn latest_id(&self) -> anyhow::Result<Option<u64>> {
            // What the NEXT read would see — the signal that rows landed
            // after the pass that just published. Diverges from the real
            // reader, which observes committed-now: this fake can never show
            // probe and read at the same instant, so the same-instant case is
            // untestable here and covered by the `<=` comparison alone.
            let versions = self.versions.lock().await;
            Ok(versions[0].iter().map(|r| r.id).max())
        }
    }

    /// Clock whose every call advances one second, so each pass gets a fresh
    /// `synced_at` and the writer's watermark can only move forward.
    fn ticking_clock(start: &str) -> anyhow::Result<impl FnMut() -> DateTime> {
        let mut t = parse(start)?;
        Ok(move || {
            t += chrono::Duration::seconds(1);
            t
        })
    }

    #[tokio::test]
    async fn a_quiet_log_publishes_exactly_once() -> anyhow::Result<()> {
        let reader = GrowingReader::new(vec![vec![row(7, "2026-07-01 10:00:00")?]]);
        let writer = FakeWriter::default();

        let summary = run_sync_until_quiescent(
            &reader,
            &writer,
            ticking_clock("2026-07-29 12:00:00")?,
            false,
        )
        .await
        .map_err(|e| match e {
            SyncError::EmptyLog(msg) => anyhow::anyhow!(msg),
            SyncError::Failed(e) => e,
        })?;

        let SyncOutcome::Published(summary) = summary else {
            anyhow::bail!("a stale snapshot must be republished");
        };
        assert_eq!(summary.max_id, Some(7));
        assert_eq!(writer.replace_count().await, 1);
        Ok(())
    }

    #[tokio::test]
    async fn a_current_snapshot_skips_the_copy_entirely() -> anyhow::Result<()> {
        let reader = GrowingReader::new(vec![vec![row(7, "2026-07-01 10:00:00")?]]);
        let writer = FakeWriter::default().with_published(7).await;

        let outcome = run_sync_until_quiescent(
            &reader,
            &writer,
            ticking_clock("2026-07-29 12:00:00")?,
            false,
        )
        .await
        .map_err(|e| match e {
            SyncError::EmptyLog(msg) => anyhow::anyhow!(msg),
            SyncError::Failed(e) => e,
        })?;

        assert_eq!(outcome, SyncOutcome::AlreadyCurrent { max_id: Some(7) });
        assert_eq!(writer.replace_count().await, 0);
        Ok(())
    }

    #[tokio::test]
    async fn a_snapshot_behind_the_log_is_republished() -> anyhow::Result<()> {
        let reader = GrowingReader::new(vec![vec![
            row(7, "2026-07-01 10:00:00")?,
            row(9, "2026-07-02 08:00:00")?,
        ]]);
        let writer = FakeWriter::default().with_published(7).await;

        run_sync_until_quiescent(
            &reader,
            &writer,
            ticking_clock("2026-07-29 12:00:00")?,
            false,
        )
        .await
        .map_err(|e| match e {
            SyncError::EmptyLog(msg) => anyhow::anyhow!(msg),
            SyncError::Failed(e) => e,
        })?;

        assert_eq!(writer.replace_count().await, 1);
        Ok(())
    }

    #[tokio::test]
    async fn force_republishes_a_current_snapshot() -> anyhow::Result<()> {
        let reader = GrowingReader::new(vec![vec![row(7, "2026-07-01 10:00:00")?]]);
        let writer = FakeWriter::default().with_published(7).await;

        run_sync_until_quiescent(
            &reader,
            &writer,
            ticking_clock("2026-07-29 12:00:00")?,
            true,
        )
        .await
        .map_err(|e| match e {
            SyncError::EmptyLog(msg) => anyhow::anyhow!(msg),
            SyncError::Failed(e) => e,
        })?;

        assert_eq!(
            writer.replace_count().await,
            1,
            "--force republishes deliberately"
        );
        Ok(())
    }

    #[tokio::test]
    async fn rows_landing_during_a_pass_are_published_by_the_next_one() -> anyhow::Result<()> {
        let v1 = vec![row(7, "2026-07-01 10:00:00")?];
        let v2 = vec![
            row(7, "2026-07-01 10:00:00")?,
            row(9, "2026-07-02 08:00:00")?,
        ];
        let reader = GrowingReader::new(vec![v1, v2]);
        let writer = FakeWriter::default();

        let summary = run_sync_until_quiescent(
            &reader,
            &writer,
            ticking_clock("2026-07-29 12:00:00")?,
            false,
        )
        .await
        .map_err(|e| match e {
            SyncError::EmptyLog(msg) => anyhow::anyhow!(msg),
            SyncError::Failed(e) => e,
        })?;

        let SyncOutcome::Published(summary) = summary else {
            anyhow::bail!("late rows must be republished");
        };
        assert_eq!(
            summary.max_id,
            Some(9),
            "the snapshot carries the late rows"
        );
        assert_eq!(writer.replace_count().await, 2);
        Ok(())
    }

    #[tokio::test]
    async fn a_log_that_never_settles_stops_at_the_pass_cap() -> anyhow::Result<()> {
        // More versions than the cap allows passes: the loop must stop, not chase.
        let versions: Vec<Vec<PersonsLogRow>> = (0..10)
            .map(|n| {
                (0..=n)
                    .map(|id| row(id + 1, "2026-07-01 10:00:00"))
                    .collect()
            })
            .collect::<anyhow::Result<_>>()?;
        let reader = GrowingReader::new(versions);
        let writer = FakeWriter::default();

        run_sync_until_quiescent(
            &reader,
            &writer,
            ticking_clock("2026-07-29 12:00:00")?,
            false,
        )
        .await
        .map_err(|e| match e {
            SyncError::EmptyLog(msg) => anyhow::anyhow!(msg),
            SyncError::Failed(e) => e,
        })?;

        assert_eq!(
            writer.replace_count().await,
            1 + MAX_QUIESCENCE_PASSES as usize,
            "extra passes beyond the first are capped"
        );
        Ok(())
    }

    #[tokio::test]
    async fn each_pass_stamps_a_fresh_synced_at() -> anyhow::Result<()> {
        let v1 = vec![row(1, "2026-07-01 10:00:00")?];
        let v2 = vec![
            row(1, "2026-07-01 10:00:00")?,
            row(2, "2026-07-02 08:00:00")?,
        ];
        let reader = GrowingReader::new(vec![v1, v2]);
        let writer = FakeWriter::default();

        run_sync_until_quiescent(
            &reader,
            &writer,
            ticking_clock("2026-07-29 12:00:00")?,
            false,
        )
        .await
        .map_err(|e| match e {
            SyncError::EmptyLog(msg) => anyhow::anyhow!(msg),
            SyncError::Failed(e) => e,
        })?;

        let stamps = writer.synced_ats().await;
        assert_eq!(stamps.len(), 2);
        assert!(stamps[1] > stamps[0], "the watermark must move forward");
        Ok(())
    }

    #[tokio::test]
    async fn an_empty_log_refusal_ends_the_loop_immediately() -> anyhow::Result<()> {
        let reader = GrowingReader::new(vec![Vec::new(), vec![row(1, "2026-07-01 10:00:00")?]]);
        let writer = FakeWriter::default();

        let result = run_sync_until_quiescent(
            &reader,
            &writer,
            ticking_clock("2026-07-29 12:00:00")?,
            false,
        )
        .await;

        assert!(matches!(result, Err(SyncError::EmptyLog(_))));
        assert_eq!(writer.replace_count().await, 0);
        Ok(())
    }

    #[tokio::test]
    async fn an_empty_log_is_refused_without_writing_anything() -> anyhow::Result<()> {
        let writer = FakeWriter::default();

        let result = run_sync(
            &FakeReader(Vec::new()),
            &writer,
            parse("2026-07-29 12:00:00")?,
            false,
        )
        .await;

        let Err(SyncError::EmptyLog(msg)) = result else {
            anyhow::bail!("an empty log must be refused");
        };
        assert!(msg.contains("--force"), "{msg}");
        // The point of the guard: the populated snapshot survives untouched.
        assert_eq!(writer.replace_count().await, 0);
        Ok(())
    }

    #[tokio::test]
    async fn force_publishes_the_empty_snapshot() -> anyhow::Result<()> {
        let writer = FakeWriter::default();

        let summary = run_sync(
            &FakeReader(Vec::new()),
            &writer,
            parse("2026-07-29 12:00:00")?,
            true,
        )
        .await
        .map_err(|e| match e {
            SyncError::EmptyLog(msg) => anyhow::anyhow!(msg),
            SyncError::Failed(e) => e,
        })?;

        assert_eq!(summary.rows, 0);
        assert_eq!(summary.max_id, None);
        assert_eq!(summary.max_created_at, None);
        // The writer still runs — a deliberate empty snapshot clears the table.
        assert_eq!(writer.last().await.map(|(rows, _)| rows), Some(0));
        Ok(())
    }

    #[tokio::test]
    async fn summarizes_rows_and_watermarks() -> anyhow::Result<()> {
        let reader = FakeReader(vec![
            row(7, "2026-07-01 10:00:00")?,
            row(3, "2026-07-20 09:30:00")?,
        ]);
        let writer = FakeWriter::default();
        let now = parse("2026-07-29 12:00:00")?;

        let summary = run_sync(&reader, &writer, now, false)
            .await
            .map_err(|e| match e {
                SyncError::EmptyLog(msg) => anyhow::anyhow!(msg),
                SyncError::Failed(e) => e,
            })?;

        assert_eq!(summary.rows, 2);
        assert_eq!(summary.max_id, Some(7));
        assert_eq!(
            summary.max_created_at.as_deref(),
            Some("2026-07-20T09:30:00.000000")
        );
        assert_eq!(summary.synced_at, "2026-07-29T12:00:00.000000");
        assert_eq!(writer.last().await, Some((2, now)));
        Ok(())
    }

    #[tokio::test]
    async fn writer_failure_propagates() -> anyhow::Result<()> {
        struct FailingWriter;
        #[async_trait]
        impl IdentityPersonsWriter for FailingWriter {
            async fn replace(&self, _: &[PersonsLogRow], _: DateTime) -> anyhow::Result<()> {
                anyhow::bail!("clickhouse is down")
            }
            async fn published_max_id(&self) -> anyhow::Result<Option<u64>> {
                Ok(None)
            }
        }
        let reader = FakeReader(vec![row(1, "2026-07-01 10:00:00")?]);
        let result = run_sync(
            &reader,
            &FailingWriter,
            parse("2026-07-29 12:00:00")?,
            false,
        )
        .await;
        assert!(result.is_err());
        Ok(())
    }
}
