//! CLI policy-publish runner — the engine behind the `publish-policy`
//! subcommand.
//!
//! Publishes the registry's current per-definition policy into ClickHouse so
//! the query path enforces it without calling this service. Same execution
//! model as the seed/sync/reconcile runners: Helm `CronJob` or manual Job;
//! only GET journal routes exist on the API.
//!
//! One run: advisory lock → zombie sweep → `operations` journal row → read →
//! short-circuit check → publish → journal completed/failed.
//!
//! Two deliberate divergences from persons-sync:
//!
//! - An EMPTY policy set publishes an empty snapshot rather than refusing.
//!   The registry is legitimately empty before the first reconcile, and an
//!   empty snapshot fails CLOSED downstream (no policy row means the query
//!   path may not compare), so the destructive-empty reasoning that guards
//!   the persons log does not apply here.
//! - Unchanged policy short-circuits instead of re-publishing. The check
//!   consults BOTH the last journalled activation AND the live relation's row
//!   count: a checksum alone would skip forever against a ClickHouse that had
//!   been wiped or re-pointed since that activation.

use std::time::Duration;

use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::GearConfig;
use crate::infra::db::person_attributes_repo::{self, CurrentPolicyRow};
use crate::infra::db::{self, ops_repo, seed_repo};
use crate::infra::policy_snapshot::ClickHousePolicySnapshotWriter;
use crate::seed_runner::{SYSTEM_AUTHOR, resolve_tenant};

const PUBLISH_TIMEOUT: Duration = Duration::from_mins(5);
const RUN_TIMEOUT: Duration = Duration::from_mins(7);
const ZOMBIE_CUTOFF_HOURS: i64 = 1;

/// Why a publish run did not complete — mapped to the shared exit-code
/// scheme (0 ok / 1 failed / 2 lock busy / 3 guard).
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

/// Accounting of one publish run, journalled as `summary_json` and read back
/// by the next run's short-circuit check.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishSummary {
    pub rows: u64,
    pub checksum: String,
    /// True when the run verified the published snapshot already matched and
    /// wrote nothing. Journalled anyway, so the journal still answers "did
    /// the schedule fire".
    pub skipped: bool,
}

/// Run one CLI policy publish end to end.
///
/// # Errors
///
/// [`PublishRunError`] — lock busy or a failed run.
pub async fn run(config: &GearConfig) -> Result<PublishSummary, PublishRunError> {
    let db = db::connect(&config.database_url).await?;

    // The snapshot spans every tenant in the registry, but its journal row
    // needs one real tenant (the GET journal routes are tenant-scoped) —
    // same resolution rule as seed/sync/reconcile.
    let distinct = seed_repo::distinct_tenants(&db, 2).await?;
    let tenant = match resolve_tenant(&config.tenant_default_id, &distinct) {
        Ok(t) => t,
        Err(msg) => return Err(PublishRunError::Failed(anyhow::anyhow!(msg))),
    };

    let Some(lock) = db::PolicyPublishLockGuard::try_acquire(&config.database_url).await? else {
        return Err(PublishRunError::LockBusy);
    };

    let result = tokio::time::timeout(RUN_TIMEOUT, run_locked(&db, config, tenant))
        .await
        .unwrap_or_else(|_| {
            Err(PublishRunError::Failed(anyhow::anyhow!(
                "policy publish timed out after {}s inside the lock-held critical section",
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
) -> Result<PublishSummary, PublishRunError> {
    let cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::hours(ZOMBIE_CUTOFF_HOURS);
    match ops_repo::sweep_zombies(db, cutoff).await {
        Ok(n) if n > 0 => tracing::warn!(swept = n, "policy-publish: reclaimed zombie operations"),
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "policy-publish: zombie sweep failed"),
    }

    // Read the previous activation BEFORE journalling this run, or the lookup
    // would find this run's own queued row.
    let previous = last_activation(db, tenant).await;

    let operation_id = Uuid::now_v7();
    let request_json = serde_json::json!({ "trigger": "cli" }).to_string();
    ops_repo::enqueue(
        db,
        operation_id,
        ops_repo::PERSON_ATTRIBUTES_POLICY_PUBLISH_OP,
        tenant,
        SYSTEM_AUTHOR,
        Some(&request_json),
    )
    .await?;
    ops_repo::try_start(db, operation_id).await?;
    tracing::info!(%operation_id, %tenant, "policy-publish: cli run started");

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

/// The most recent completed publish activation for this tenant, if any.
/// A malformed or missing summary reads as "no previous activation", which
/// costs one redundant publish — never a wrong skip.
async fn last_activation(db: &DatabaseConnection, tenant: Uuid) -> Option<PublishSummary> {
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

/// A skip is only safe when the journal AND the live relation agree: the
/// journal proves the content is unchanged, the row count proves the
/// snapshot the journal describes is still the one published. A ClickHouse
/// that lost the relation reads as "not published" and re-publishes.
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

/// Content hash of the policy set. Stable across runs that changed nothing
/// and sensitive to every published field, so an edit anywhere re-publishes.
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

    #[tokio::test]
    async fn default_config_fails_cleanly() {
        let cfg = crate::config::GearConfig::default();
        let err = run(&cfg).await;
        assert!(matches!(err, Err(PublishRunError::Failed(_))));
    }
}
