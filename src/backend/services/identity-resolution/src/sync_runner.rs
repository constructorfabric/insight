//! CLI persons-sync runner — the engine behind the `sync` subcommand.
//!
//! Copies the MariaDB `persons` observation log into ClickHouse
//! `identity.identity_persons` (full snapshot, atomic swap — see
//! `infra::identity_persons`) so the metrics dbt builds can resolve
//! `email -> person_id`. Nothing schedules it on its own: every seed run and
//! every applied operator correction invokes this runner as its publish step,
//! and the `sync` subcommand remains as the manual repair tool for a snapshot
//! that has fallen behind them — no HTTP trigger, no auth; only the GET
//! journal routes remain on the API.
//!
//! One run: advisory lock → zombie sweep → `operations` journal row → log
//! read → guard → snapshot replace → journal completed/failed.
//!
//! Concurrency: runs serialize on a GLOBAL MariaDB `GET_LOCK`
//! (`infra::db::SyncLockGuard` — global, not per-tenant: the sync copies the
//! whole log). This is the actual serialization of the publish step; the
//! writer's `_synced_at` watermark guard is a backstop for anything that
//! bypasses the runner. A concurrent run waits up to [`LOCK_WAIT_SECS`] for
//! the holder, then reports [`SyncRunError::LockBusy`] (exit code 2) — and
//! the holder's quiescence re-check means its snapshot already covers every
//! row committed before a waiter gave up.
//!
//! Guard (overridable with `--force`, recorded as a `failed` operation so
//! the journal explains why nothing was published): an EMPTY `persons` log.
//! An empty read usually means a misconfigured database or a wiped stand,
//! not "nobody exists" — and publishing it would atomically erase a
//! populated mirror (the destructive-zero-rows lesson of the seed, #1550).
//! A deliberate wipe is what `--force` is for; the domain layer itself
//! treats an empty snapshot as valid.

use std::time::Duration;

use uuid::Uuid;

use crate::config::GearConfig;
use crate::domain::sync_service::{SyncError, SyncOutcome, run_sync_until_quiescent};
use crate::infra::db::{self, ops_repo, persons_log_repo::MariaDbPersonsLogReader, seed_repo};
use crate::infra::identity_persons::ClickHouseIdentityPersonsWriter;
use crate::seed_runner::{SYSTEM_AUTHOR, resolve_tenant};

/// Upper bound on the read + replace — a healthy run is seconds; this only
/// trips on a real MariaDB/ClickHouse stall, failing the Job instead of
/// wedging it into the `CronJob`'s next tick.
const SYNC_TIMEOUT: Duration = Duration::from_mins(5);

/// Backstop over the WHOLE lock-held critical section (sweep + journal
/// writes are outside [`SYNC_TIMEOUT`]'s scope). A run cut off here may
/// leave its `operations` row `running`; the next run's zombie sweep
/// reclaims it.
const RUN_TIMEOUT: Duration = Duration::from_mins(7);

/// How stale a `queued`/`running` operation must be before the pre-run
/// sweep reclaims it (same convention as the seed runner).
const ZOMBIE_CUTOFF_HOURS: i64 = 1;

/// How long a run waits for the advisory lock before reporting `LockBusy`.
/// A healthy publish is seconds even with its quiescence re-copies, so a
/// holder outliving this wait is wedged, not slow. Each waiter parks one
/// MariaDB session for at most this long — keep it well under [`RUN_TIMEOUT`].
const LOCK_WAIT_SECS: u32 = 15;

/// Why a sync run did not complete — the `sync` subcommand maps each variant
/// to a distinct exit code (0 ok / 1 failed / 2 lock busy / 3 guard), same
/// scheme as the seed.
#[derive(Debug)]
pub enum SyncRunError {
    /// Another run holds the global sync advisory lock.
    LockBusy,
    /// The guard refused the run (operator-facing message, persisted
    /// verbatim as the operation's `error_message`).
    Guard(String),
    /// The run itself failed (connect, read, replace, or journal write).
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for SyncRunError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed(e)
    }
}

/// Run one CLI persons-sync end to end. See the module docs for the shape.
///
/// # Errors
///
/// [`SyncRunError`] — lock busy, guard refusal, or a failed run.
pub async fn run(config: &GearConfig, force: bool) -> Result<SyncOutcome, SyncRunError> {
    let db = db::connect(&config.database_url).await?;

    // The sync itself is tenant-agnostic (whole-log copy), but its journal
    // row needs a real tenant: the GET journal routes are tenant-scoped, so
    // rows under a made-up tenant would be invisible to the admins who need
    // them. Same resolution rule as the seed (configured || sole-in-log).
    let distinct = seed_repo::distinct_tenants(&db, 2).await?;
    let tenant = match resolve_tenant(&config.tenant_default_id, &distinct) {
        Ok(t) => t,
        Err(msg) => return Err(SyncRunError::Failed(anyhow::anyhow!(msg))),
    };

    let Some(lock) = db::SyncLockGuard::acquire(&config.database_url, LOCK_WAIT_SECS).await? else {
        return Err(SyncRunError::LockBusy);
    };

    let result = tokio::time::timeout(RUN_TIMEOUT, run_locked(&db, config, tenant, force))
        .await
        .unwrap_or_else(|_| {
            Err(SyncRunError::Failed(anyhow::anyhow!(
                "persons-sync run timed out after {}s inside the lock-held critical section",
                RUN_TIMEOUT.as_secs()
            )))
        });

    lock.release().await;
    result
}

/// Everything that happens while the advisory lock is held: zombie sweep,
/// journal row, guarded sync, journal resolution.
async fn run_locked(
    db: &sea_orm::DatabaseConnection,
    config: &GearConfig,
    tenant: Uuid,
    force: bool,
) -> Result<SyncOutcome, SyncRunError> {
    // Reclaim rows a killed run left behind. Log-only failure: a broken
    // sweep must not block the sync itself.
    let cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::hours(ZOMBIE_CUTOFF_HOURS);
    match ops_repo::sweep_zombies(db, cutoff).await {
        Ok(n) if n > 0 => tracing::warn!(swept = n, "persons-sync: reclaimed zombie operations"),
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "persons-sync: zombie sweep failed"),
    }

    // Journal row first, so every later failure (guard included) is recorded
    // and visible over the GET /v1/persons-sync endpoints.
    let operation_id = Uuid::now_v7();
    let request_json = serde_json::json!({ "trigger": "cli", "force": force }).to_string();
    ops_repo::enqueue(
        db,
        operation_id,
        ops_repo::PERSONS_SYNC_OP,
        tenant,
        SYSTEM_AUTHOR,
        Some(&request_json),
    )
    .await?;
    ops_repo::try_start(db, operation_id).await?;
    tracing::info!(%operation_id, %tenant, force, "persons-sync: cli run started");

    match guarded_sync(db, config, force).await {
        Ok(summary) => {
            let summary_json = serde_json::to_string(&summary).unwrap_or_else(|_| "{}".to_owned());
            ops_repo::complete(db, operation_id, &summary_json).await?;
            tracing::info!(%operation_id, ?summary, "persons-sync: completed");
            Ok(summary)
        }
        Err(SyncRunError::Guard(msg)) => {
            // Deliberate operator-facing text — safe to persist verbatim.
            tracing::warn!(%operation_id, %msg, "persons-sync: refused by guard");
            if let Err(e) = ops_repo::fail(db, operation_id, &msg).await {
                tracing::error!(error = %e, %operation_id, "fail update failed");
            }
            Err(SyncRunError::Guard(msg))
        }
        Err(e) => {
            // Persist only a generic message: `error_message` is returned
            // verbatim by the GET endpoints, so raw driver/anyhow text must
            // not leak to callers.
            if let Err(e2) =
                ops_repo::fail(db, operation_id, "persons-sync failed; see job logs").await
            {
                tracing::error!(error = %e2, %operation_id, "fail update failed");
            }
            Err(e)
        }
    }
}

/// Log read → guard → snapshot replace, bounded by [`SYNC_TIMEOUT`].
async fn guarded_sync(
    db: &sea_orm::DatabaseConnection,
    config: &GearConfig,
    force: bool,
) -> Result<SyncOutcome, SyncRunError> {
    let reader = MariaDbPersonsLogReader::new(db);
    let writer = ClickHouseIdentityPersonsWriter::connect(
        &config.clickhouse_url,
        &config.clickhouse_user,
        &config.clickhouse_password,
    );

    let run = async {
        // The empty-log guard lives with the rows it judges (see `run_sync`), so
        // a seed emptying the log mid-run cannot slip past a stale count.
        run_sync_until_quiescent(&reader, &writer, || chrono::Utc::now().naive_utc(), force)
            .await
            .map_err(|e| match e {
                SyncError::EmptyLog(msg) => SyncRunError::Guard(msg),
                SyncError::Failed(e) => SyncRunError::Failed(e),
            })
    };

    tokio::time::timeout(SYNC_TIMEOUT, run)
        .await
        .unwrap_or_else(|_| {
            Err(SyncRunError::Failed(anyhow::anyhow!(
                "persons-sync timed out after {}s",
                SYNC_TIMEOUT.as_secs()
            )))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_config_fails_cleanly() {
        let cfg = crate::config::GearConfig::default();
        let err = run(&cfg, false).await;
        assert!(matches!(err, Err(SyncRunError::Failed(_))));
    }
}
