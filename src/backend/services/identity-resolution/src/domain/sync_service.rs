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
}

/// Replaces ClickHouse `identity.identity_persons` with a new snapshot.
#[async_trait]
pub trait IdentityPersonsWriter: Send + Sync {
    /// Load `rows` (stamped with `synced_at`) into a staging table and swap it
    /// in atomically. Must leave the previous snapshot untouched on any failure.
    async fn replace(&self, rows: &[PersonsLogRow], synced_at: DateTime) -> anyhow::Result<()>;
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
    }

    /// Records what was written; async `Mutex` because the trait takes `&self`.
    #[derive(Default)]
    struct FakeWriter {
        written: Mutex<Option<(usize, DateTime)>>,
    }
    #[async_trait]
    impl IdentityPersonsWriter for FakeWriter {
        async fn replace(&self, rows: &[PersonsLogRow], synced_at: DateTime) -> anyhow::Result<()> {
            *self.written.lock().await = Some((rows.len(), synced_at));
            Ok(())
        }
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
        assert!(writer.written.lock().await.is_none());
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
        assert_eq!(writer.written.lock().await.map(|(rows, _)| rows), Some(0));
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
        let written = writer.written.lock().await.take();
        assert_eq!(written, Some((2, now)));
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
