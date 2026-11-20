use std::time::Duration;

use uuid::Uuid;

use crate::config::GearConfig;
use crate::domain::sync_service::{SyncError, SyncOutcome, run_sync_until_quiescent};
use crate::infra::db::{self, ops_repo, persons_log_repo::MariaDbPersonsLogReader, seed_repo};
use crate::infra::identity_persons::ClickHouseIdentityPersonsWriter;
use crate::seed_runner::{SYSTEM_AUTHOR, resolve_tenant};

const SYNC_TIMEOUT: Duration = Duration::from_mins(5);

const RUN_TIMEOUT: Duration = Duration::from_mins(7);

const ZOMBIE_CUTOFF_HOURS: i64 = 1;

const LOCK_WAIT_SECS: u32 = 15;

#[derive(Debug)]
pub enum SyncRunError {
    LockBusy,
    Guard(String),
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for SyncRunError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed(e)
    }
}

pub async fn run(config: &GearConfig, force: bool) -> Result<SyncOutcome, SyncRunError> {
    let db = db::connect(&config.database_url).await?;
    db::assert_schema_compatible(&db).await?;

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

async fn run_locked(
    db: &sea_orm::DatabaseConnection,
    config: &GearConfig,
    tenant: Uuid,
    force: bool,
) -> Result<SyncOutcome, SyncRunError> {
    let cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::hours(ZOMBIE_CUTOFF_HOURS);
    match ops_repo::sweep_zombies(db, cutoff).await {
        Ok(n) if n > 0 => tracing::warn!(swept = n, "persons-sync: reclaimed zombie operations"),
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "persons-sync: zombie sweep failed"),
    }

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
            tracing::warn!(%operation_id, %msg, "persons-sync: refused by guard");
            if let Err(e) = ops_repo::fail(db, operation_id, &msg).await {
                tracing::error!(error = %e, %operation_id, "fail update failed");
            }
            Err(SyncRunError::Guard(msg))
        }
        Err(e) => {
            if let Err(e2) =
                ops_repo::fail(db, operation_id, "persons-sync failed; see job logs").await
            {
                tracing::error!(error = %e2, %operation_id, "fail update failed");
            }
            Err(e)
        }
    }
}

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
        config.topology(),
    );

    let run = async {
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
