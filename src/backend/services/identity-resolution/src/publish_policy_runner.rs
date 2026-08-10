use std::time::Duration;

use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::GearConfig;
use crate::infra::db::person_attributes_repo::{self, CurrentPolicyRow};
use crate::infra::db::{self, ops_repo, seed_repo};
use crate::infra::policy_snapshot::ClickHousePolicySnapshotWriter;
use crate::seed_runner::{SYSTEM_AUTHOR, resolve_tenant};

const PUBLISH_TIMEOUT: Duration = Duration::from_mins(5);
const RUN_TIMEOUT: Duration = Duration::from_mins(7);
const ZOMBIE_CUTOFF_HOURS: i64 = 1;
const ABORTED_BY_SHUTDOWN: &str = "policy publish aborted by server shutdown";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishTrigger {
    Cli,
    Http,
}

impl PublishTrigger {
    pub(crate) fn request_json(self) -> &'static str {
        match self {
            Self::Cli => r#"{"trigger":"cli"}"#,
            Self::Http => r#"{"trigger":"http"}"#,
        }
    }
}

#[derive(Debug)]
pub enum PublishRunError {
    LockBusy,
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for PublishRunError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed(e)
    }
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishSummary {
    pub rows: u64,
    pub checksum: String,
    pub skipped: bool,
}

pub(crate) async fn resolve_journal_tenant(
    db: &DatabaseConnection,
    config: &GearConfig,
) -> anyhow::Result<Uuid> {
    let distinct = seed_repo::distinct_tenants(db, 2).await?;
    resolve_tenant(&config.tenant_default_id, &distinct).map_err(|msg| anyhow::anyhow!(msg))
}

pub(crate) async fn enqueue_run(
    db: &DatabaseConnection,
    tenant: Uuid,
    author: Uuid,
    trigger: PublishTrigger,
) -> anyhow::Result<Uuid> {
    let operation_id = Uuid::now_v7();
    ops_repo::enqueue(
        db,
        operation_id,
        ops_repo::PERSON_ATTRIBUTES_POLICY_PUBLISH_OP,
        tenant,
        author,
        Some(trigger.request_json()),
    )
    .await?;
    Ok(operation_id)
}

pub async fn run(config: &GearConfig) -> Result<PublishSummary, PublishRunError> {
    let db = db::connect(&config.database_url).await?;
    let tenant = resolve_journal_tenant(&db, config).await?;

    let Some(lock) = db::PolicyPublishLockGuard::try_acquire(&config.database_url).await? else {
        return Err(PublishRunError::LockBusy);
    };
    let operation_id = enqueue_run(&db, tenant, SYSTEM_AUTHOR, PublishTrigger::Cli).await?;

    let result = run_bounded(&db, config, &lock, tenant, operation_id).await;

    lock.release().await;
    result
}

pub(crate) async fn run_detached(
    db: DatabaseConnection,
    config: GearConfig,
    cancel: CancellationToken,
    tenant: Uuid,
    operation_id: Uuid,
    lock: db::PolicyPublishLockGuard,
) {
    let outcome = tokio::select! {
        biased;
        () = cancel.cancelled() => None,
        result = run_bounded(&db, &config, &lock, tenant, operation_id) => Some(result),
    };

    match outcome {
        Some(Ok(summary)) => {
            tracing::info!(%operation_id, ?summary, "policy-publish: triggered run finished");
        }
        Some(Err(e)) => {
            tracing::error!(error = ?e, %operation_id, "policy-publish: triggered run failed");
        }
        None => {
            tracing::warn!(%operation_id, "policy-publish: run cut short by shutdown");
            if let Err(e) = ops_repo::fail(&db, operation_id, ABORTED_BY_SHUTDOWN).await {
                tracing::error!(error = %e, %operation_id, "shutdown fail-update failed");
            }
            return;
        }
    }

    lock.release().await;
}

async fn run_bounded(
    db: &DatabaseConnection,
    config: &GearConfig,
    _lock: &db::PolicyPublishLockGuard,
    tenant: Uuid,
    operation_id: Uuid,
) -> Result<PublishSummary, PublishRunError> {
    tokio::time::timeout(RUN_TIMEOUT, run_locked(db, config, tenant, operation_id))
        .await
        .unwrap_or_else(|_| {
            Err(PublishRunError::Failed(anyhow::anyhow!(
                "policy publish timed out after {}s inside the lock-held critical section",
                RUN_TIMEOUT.as_secs()
            )))
        })
}

async fn run_locked(
    db: &DatabaseConnection,
    config: &GearConfig,
    tenant: Uuid,
    operation_id: Uuid,
) -> Result<PublishSummary, PublishRunError> {
    let cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::hours(ZOMBIE_CUTOFF_HOURS);
    match ops_repo::sweep_zombies(db, cutoff).await {
        Ok(n) if n > 0 => tracing::warn!(swept = n, "policy-publish: reclaimed zombie operations"),
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "policy-publish: zombie sweep failed"),
    }

    let previous = last_activation(db, tenant).await;
    ops_repo::try_start(db, operation_id).await?;
    tracing::info!(%operation_id, %tenant, "policy-publish: run started");

    match guarded_publish(db, config, previous.as_ref()).await {
        Ok(summary) => {
            let summary_json = serde_json::to_string(&summary).unwrap_or_else(|_| "{}".to_owned());
            ops_repo::complete(db, operation_id, &summary_json).await?;
            tracing::info!(%operation_id, ?summary, "policy-publish: completed");
            Ok(summary)
        }
        Err(e) => {
            if let Err(e2) =
                ops_repo::fail(db, operation_id, "policy publish failed; see job logs").await
            {
                tracing::error!(error = %e2, %operation_id, "fail update failed");
            }
            Err(e)
        }
    }
}

async fn last_activation(db: &DatabaseConnection, tenant: Uuid) -> Option<PublishSummary> {
    // INVARIANT: filtering on `completed` is what keeps this run from reading its own row —
    // it is `queued` until this same run completes it.
    let ops = ops_repo::list(
        db,
        tenant,
        Some(ops_repo::PERSON_ATTRIBUTES_POLICY_PUBLISH_OP),
        Some(ops_repo::OperationStatus::Completed),
        1,
    )
    .await
    .inspect_err(|e| tracing::warn!(error = %e, "policy-publish: activation lookup failed"))
    .ok()?;
    let summary_json = ops.into_iter().next()?.summary_json?;
    serde_json::from_str(&summary_json).ok()
}

async fn guarded_publish(
    db: &DatabaseConnection,
    config: &GearConfig,
    previous: Option<&PublishSummary>,
) -> Result<PublishSummary, PublishRunError> {
    let writer = ClickHousePolicySnapshotWriter::connect(
        &config.clickhouse_url,
        &config.clickhouse_user,
        &config.clickhouse_password,
    );

    let run = async {
        let policies = person_attributes_repo::current_policies(db).await?;
        let checksum = checksum(&policies);
        let rows = policies.len() as u64;

        if is_already_published(previous, &checksum, rows, &writer).await {
            tracing::info!(rows, %checksum, "policy-publish: snapshot already current, skipping");
            return Ok(PublishSummary {
                rows,
                checksum,
                skipped: true,
            });
        }

        writer.replace(&policies, chrono::Utc::now()).await?;
        Ok(PublishSummary {
            rows,
            checksum,
            skipped: false,
        })
    };

    tokio::time::timeout(PUBLISH_TIMEOUT, run)
        .await
        .unwrap_or_else(|_| {
            Err(PublishRunError::Failed(anyhow::anyhow!(
                "policy publish timed out after {}s",
                PUBLISH_TIMEOUT.as_secs()
            )))
        })
}

async fn is_already_published(
    previous: Option<&PublishSummary>,
    checksum: &str,
    rows: u64,
    writer: &ClickHousePolicySnapshotWriter,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if previous.checksum != checksum || previous.rows != rows {
        return false;
    }
    match writer.published_row_count().await {
        Ok(published) => published == rows,
        Err(e) => {
            tracing::warn!(error = %e, "policy-publish: published-count check failed; publishing");
            false
        }
    }
}

fn checksum(policies: &[CurrentPolicyRow]) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    for p in policies {
        p.definition_id.hash(&mut hasher);
        p.insight_tenant_id.hash(&mut hasher);
        p.insight_source_type.hash(&mut hasher);
        p.insight_source_id.hash(&mut hasher);
        p.source_field_id.hash(&mut hasher);
        p.revision.hash(&mut hasher);
        p.label_override.hash(&mut hasher);
        p.sensitivity_class.hash(&mut hasher);
        p.grouping_enabled.hash(&mut hasher);
        p.comparison_enabled.hash(&mut hasher);
        p.value_mode.as_db().hash(&mut hasher);
        p.retired.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::person_attributes_repo::ValueMode;

    fn policy(field: &str, comparison_enabled: bool) -> CurrentPolicyRow {
        CurrentPolicyRow {
            definition_id: Uuid::from_u128(1),
            insight_tenant_id: "t".to_owned(),
            insight_source_type: "bamboohr".to_owned(),
            insight_source_id: "hr-main".to_owned(),
            source_field_id: field.to_owned(),
            revision: 1,
            label_override: None,
            sensitivity_class: None,
            grouping_enabled: true,
            comparison_enabled,
            value_mode: ValueMode::Single,
            retired: false,
        }
    }

    #[test]
    fn checksum_is_stable_for_an_unchanged_policy_set() {
        let set = vec![policy("jobTitle", false), policy("department", false)];
        assert_eq!(checksum(&set), checksum(&set.clone()));
    }

    #[test]
    fn checksum_changes_when_any_published_field_changes() {
        let before = vec![policy("jobTitle", false)];
        let after = vec![policy("jobTitle", true)];
        assert_ne!(
            checksum(&before),
            checksum(&after),
            "enabling comparison must re-publish"
        );
    }

    #[test]
    fn checksum_of_an_empty_registry_is_stable_and_distinct() {
        let empty = checksum(&[]);
        assert_eq!(empty, checksum(&[]));
        assert_ne!(empty, checksum(&[policy("jobTitle", false)]));
    }

    #[test]
    fn the_journalled_request_names_the_trigger() {
        for (trigger, expected) in [
            (PublishTrigger::Cli, r#"{"trigger":"cli"}"#),
            (PublishTrigger::Http, r#"{"trigger":"http"}"#),
        ] {
            assert_eq!(
                trigger.request_json(),
                expected,
                "journal request payload changed for {trigger:?}"
            );
        }
    }

    #[tokio::test]
    async fn default_config_fails_cleanly() {
        let cfg = crate::config::GearConfig::default();
        let err = run(&cfg).await;
        assert!(matches!(err, Err(PublishRunError::Failed(_))));
    }
}
