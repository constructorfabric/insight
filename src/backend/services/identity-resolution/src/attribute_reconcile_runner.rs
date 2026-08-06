//! CLI attribute-reconcile runner — the engine behind the
//! `reconcile-attributes` subcommand.
//!
//! Discovers person-attribute fields in the warehouse claim relation and
//! registers them in the MariaDB registry (definition + default revision-1
//! policy: grouping allowed, comparison denied). Same execution model as the
//! seed/sync runners: a Helm `CronJob` / manual Job runs
//! `identity-resolution reconcile-attributes`; only GET journal routes exist
//! on the API.
//!
//! One run: advisory lock → zombie sweep → `operations` journal row →
//! discover → guards → register → journal completed/failed.
//!
//! Guards (exit code 3, journalled as `failed` so the journal explains why
//! nothing was registered):
//! - the claims relation does not exist — this service can deploy ahead of
//!   the ingestion release that creates it; refusing beats a red `CronJob`
//!   that alerts until the other repo ships.
//! - fields were discovered but every one had empty key components — a run
//!   that green-completes registering nothing would hide a broken claims
//!   contract indefinitely (see `domain::attribute_reconcile`).

use std::time::Duration;

use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::config::GearConfig;
use crate::domain::attribute_reconcile::{
    FieldRegistrar, ReconcileError, ReconcileSummary, run_reconcile,
};
use crate::infra::attribute_claims::{ClickHouseDiscoveredFieldsReader, DiscoverOutcome};
use crate::infra::db::person_attributes_repo::{self, DefinitionKey, RegisterOutcome};
use crate::infra::db::{self, ops_repo, seed_repo};
use crate::seed_runner::{SYSTEM_AUTHOR, resolve_tenant};

const RECONCILE_TIMEOUT: Duration = Duration::from_mins(5);
const RUN_TIMEOUT: Duration = Duration::from_mins(7);
const ZOMBIE_CUTOFF_HOURS: i64 = 1;

/// Why a reconcile run did not complete — mapped to the shared exit-code
/// scheme (0 ok / 1 failed / 2 lock busy / 3 guard).
#[derive(Debug)]
pub enum ReconcileRunError {
    LockBusy,
    Guard(String),
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for ReconcileRunError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed(e)
    }
}

/// Run one CLI attribute reconciliation end to end.
///
/// # Errors
///
/// [`ReconcileRunError`] — lock busy, guard refusal, or a failed run.
pub async fn run(config: &GearConfig) -> Result<ReconcileSummary, ReconcileRunError> {
    let db = db::connect(&config.database_url).await?;

    // The run spans every tenant present in claims, but its journal row needs
    // one real tenant (the GET journal routes are tenant-scoped) — same
    // resolution rule as seed/sync.
    let distinct = seed_repo::distinct_tenants(&db, 2).await?;
    let tenant = match resolve_tenant(&config.tenant_default_id, &distinct) {
        Ok(t) => t,
        Err(msg) => return Err(ReconcileRunError::Failed(anyhow::anyhow!(msg))),
    };

    let Some(lock) = db::ReconcileLockGuard::try_acquire(&config.database_url).await? else {
        return Err(ReconcileRunError::LockBusy);
    };

    let result = tokio::time::timeout(RUN_TIMEOUT, run_locked(&db, config, tenant))
        .await
        .unwrap_or_else(|_| {
            Err(ReconcileRunError::Failed(anyhow::anyhow!(
                "attribute reconcile timed out after {}s inside the lock-held critical section",
                RUN_TIMEOUT.as_secs()
            )))
        });

    lock.release().await;
    result
}

async fn run_locked(
    db: &DatabaseConnection,
    config: &GearConfig,
    tenant: Uuid,
) -> Result<ReconcileSummary, ReconcileRunError> {
    let cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::hours(ZOMBIE_CUTOFF_HOURS);
    match ops_repo::sweep_zombies(db, cutoff).await {
        Ok(n) if n > 0 => {
            tracing::warn!(
                swept = n,
                "attribute-reconcile: reclaimed zombie operations"
            );
        }
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "attribute-reconcile: zombie sweep failed"),
    }

    let operation_id = Uuid::now_v7();
    let request_json = serde_json::json!({ "trigger": "cli" }).to_string();
    ops_repo::enqueue(
        db,
        operation_id,
        ops_repo::PERSON_ATTRIBUTES_RECONCILE_OP,
        tenant,
        SYSTEM_AUTHOR,
        Some(&request_json),
    )
    .await?;
    ops_repo::try_start(db, operation_id).await?;
    tracing::info!(%operation_id, %tenant, "attribute-reconcile: cli run started");

    match guarded_reconcile(db, config).await {
        Ok(summary) => {
            if summary.non_canonical_tenants > 0 {
                tracing::warn!(
                    count = summary.non_canonical_tenants,
                    "attribute-reconcile: fields registered under non-canonical tenant \
                     strings; the admin API cannot address them"
                );
            }
            let summary_json = serde_json::to_string(&summary).unwrap_or_else(|_| "{}".to_owned());
            ops_repo::complete(db, operation_id, &summary_json).await?;
            tracing::info!(%operation_id, ?summary, "attribute-reconcile: completed");
            Ok(summary)
        }
        Err(ReconcileRunError::Guard(msg)) => {
            tracing::warn!(%operation_id, %msg, "attribute-reconcile: refused by guard");
            if let Err(e) = ops_repo::fail(db, operation_id, &msg).await {
                tracing::error!(error = %e, %operation_id, "fail update failed");
            }
            Err(ReconcileRunError::Guard(msg))
        }
        Err(e) => {
            if let Err(e2) =
                ops_repo::fail(db, operation_id, "attribute reconcile failed; see job logs").await
            {
                tracing::error!(error = %e2, %operation_id, "fail update failed");
            }
            Err(e)
        }
    }
}

struct RepoRegistrar<'a> {
    db: &'a DatabaseConnection,
}

#[async_trait]
impl FieldRegistrar for RepoRegistrar<'_> {
    async fn register(
        &self,
        key: &DefinitionKey,
        observed_at: &str,
    ) -> anyhow::Result<RegisterOutcome> {
        person_attributes_repo::register_discovered(self.db, key, observed_at, SYSTEM_AUTHOR).await
    }
}

async fn guarded_reconcile(
    db: &DatabaseConnection,
    config: &GearConfig,
) -> Result<ReconcileSummary, ReconcileRunError> {
    let reader = ClickHouseDiscoveredFieldsReader::connect(
        &config.clickhouse_url,
        &config.clickhouse_database,
        &config.clickhouse_user,
        &config.clickhouse_password,
    );

    let run = async {
        match reader.discover_or_missing().await? {
            DiscoverOutcome::ClaimsRelationMissing => Err(ReconcileRunError::Guard(
                "silver.class_person_attribute_claims does not exist on this warehouse yet; \
                 deploy the ingestion release that creates it, then re-run"
                    .to_owned(),
            )),
            DiscoverOutcome::Fields(fields) => {
                let registrar = RepoRegistrar { db };
                run_reconcile(&PreRead(fields), &registrar)
                    .await
                    .map_err(|e| match e {
                        ReconcileError::Guard(msg) => ReconcileRunError::Guard(msg),
                        ReconcileError::Failed(e) => ReconcileRunError::Failed(e),
                    })
            }
        }
    };

    tokio::time::timeout(RECONCILE_TIMEOUT, run)
        .await
        .unwrap_or_else(|_| {
            Err(ReconcileRunError::Failed(anyhow::anyhow!(
                "attribute reconcile timed out after {}s",
                RECONCILE_TIMEOUT.as_secs()
            )))
        })
}

/// Adapter feeding already-read fields through the domain reader port, so the
/// missing-relation classification stays in the infra layer while the domain
/// loop keeps a single entry point.
struct PreRead(Vec<crate::domain::attribute_reconcile::DiscoveredField>);

#[async_trait]
impl crate::domain::attribute_reconcile::DiscoveredFieldsReader for PreRead {
    async fn discover(
        &self,
    ) -> anyhow::Result<Vec<crate::domain::attribute_reconcile::DiscoveredField>> {
        Ok(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_config_fails_cleanly() {
        let cfg = crate::config::GearConfig::default();
        let err = run(&cfg).await;
        assert!(matches!(err, Err(ReconcileRunError::Failed(_))));
    }
}
