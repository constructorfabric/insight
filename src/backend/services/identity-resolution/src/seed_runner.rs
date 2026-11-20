use std::time::Duration;

use uuid::Uuid;

use crate::config::GearConfig;
use crate::domain::roster::RosterSource;
use crate::domain::seed_service::{IdentityInputsReader, SeedSummary, seed_from_rows};
use crate::infra::db::{self, ops_repo, seed_repo};
use crate::infra::identity_inputs::ClickHouseIdentityInputsReader;
use crate::infra::metrics;

pub const SYSTEM_AUTHOR: Uuid = Uuid::nil();

pub const LINK_BY_EMAIL_MODE: &str = "link-by-email";

const SEED_TIMEOUT: Duration = Duration::from_mins(10);

const RUN_TIMEOUT: Duration = Duration::from_mins(12);

const LOCK_WAIT_SECS: u32 = 15;

const ZOMBIE_CUTOFF_HOURS: i64 = 1;

#[derive(Debug)]
pub enum SeedRunError {
    LockBusy,
    Guard(String),
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for SeedRunError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed(e)
    }
}

pub async fn run(
    config: &GearConfig,
    mode: &str,
    force: bool,
) -> Result<SeedSummary, SeedRunError> {
    if mode != LINK_BY_EMAIL_MODE {
        return Err(SeedRunError::Failed(anyhow::anyhow!(
            "unsupported mode '{mode}'; only '{LINK_BY_EMAIL_MODE}' is available"
        )));
    }
    // SAFETY: tenancy shortcut (#1550): one run seeds only the configured tenant. Looping over tenants
    // needs the `identity_inputs` tenant filter back first.
    let db = db::connect(&config.database_url).await?;
    db::assert_schema_compatible(&db).await?;

    let distinct = seed_repo::distinct_tenants(&db, 2).await?;
    let tenant = match resolve_tenant(&config.tenant_default_id, &distinct) {
        Ok(t) => t,
        Err(msg) => return Err(SeedRunError::Failed(anyhow::anyhow!(msg))),
    };
    if config.tenant_default_id.trim().is_empty() {
        tracing::warn!(
            %tenant,
            "tenant_default_id is not configured — inferred the sole tenant \
             from the persons log; configure it explicitly to silence this"
        );
    }
    let Some(lock) =
        db::SeedLockGuard::acquire(&config.database_url, tenant, LOCK_WAIT_SECS).await?
    else {
        return Err(SeedRunError::LockBusy);
    };

    let result = tokio::time::timeout(RUN_TIMEOUT, run_locked(&db, config, tenant, mode, force))
        .await
        .unwrap_or_else(|_| {
            Err(SeedRunError::Failed(anyhow::anyhow!(
                "persons-seed run timed out after {}s inside the lock-held critical section",
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
    mode: &str,
    force: bool,
) -> Result<SeedSummary, SeedRunError> {
    let cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::hours(ZOMBIE_CUTOFF_HOURS);
    match ops_repo::sweep_zombies(db, cutoff).await {
        Ok(n) if n > 0 => tracing::warn!(swept = n, "persons-seed: reclaimed zombie operations"),
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "persons-seed: zombie sweep failed"),
    }

    let operation_id = Uuid::now_v7();
    let request_json =
        serde_json::json!({ "mode": mode, "trigger": "cli", "force": force }).to_string();
    ops_repo::enqueue(
        db,
        operation_id,
        ops_repo::PERSONS_SEED_OP,
        tenant,
        SYSTEM_AUTHOR,
        Some(&request_json),
    )
    .await?;
    ops_repo::try_start(db, operation_id).await?;
    tracing::info!(%operation_id, %tenant, mode, force, "persons-seed: cli run started");

    let started = std::time::Instant::now();
    let seed_result = guarded_seed(db, config, tenant, force).await;
    let run_outcome = match &seed_result {
        Ok(_) => metrics::RunOutcome::Success,
        Err(_) => metrics::RunOutcome::Error,
    };
    metrics::record_seed_run(run_outcome, started.elapsed());

    match seed_result {
        Ok(summary) => {
            metrics::record_seed_outcomes(&summary);
            let summary_json = serde_json::to_string(&summary).unwrap_or_else(|_| "{}".to_owned());
            ops_repo::complete(db, operation_id, &summary_json).await?;
            tracing::info!(%operation_id, ?summary, "persons-seed: completed");
            Ok(summary)
        }
        Err(SeedRunError::Guard(msg)) => {
            tracing::warn!(%operation_id, %msg, "persons-seed: refused by input guard");
            if let Err(e) = ops_repo::fail(db, operation_id, &msg).await {
                tracing::error!(error = %e, %operation_id, "fail update failed");
            }
            Err(SeedRunError::Guard(msg))
        }
        Err(e) => {
            if let Err(e2) =
                ops_repo::fail(db, operation_id, "persons-seed failed; see job logs").await
            {
                tracing::error!(error = %e2, %operation_id, "fail update failed");
            }
            Err(e)
        }
    }
}

fn roster_source(config: &GearConfig) -> Option<RosterSource> {
    RosterSource::parse(&config.roster_source_type)
}

async fn guarded_seed(
    db: &sea_orm::DatabaseConnection,
    config: &GearConfig,
    tenant: Uuid,
    force: bool,
) -> Result<SeedSummary, SeedRunError> {
    let reader = ClickHouseIdentityInputsReader::connect(
        &config.clickhouse_url,
        &config.clickhouse_database,
        &config.clickhouse_user,
        &config.clickhouse_password,
    );
    let store = seed_repo::MariaDbSeedStore::new(db);
    let roster = roster_source(config);
    if let Some(roster) = roster.as_ref() {
        tracing::info!(
            roster = roster.name(),
            "persons-seed: roster source may mint addressless accounts"
        );
    } else {
        tracing::info!("persons-seed: no roster source configured; minting needs an address");
    }

    let run = async {
        let rows = reader.stream(tenant).await?;
        tracing::info!(input_rows = rows.len(), "persons-seed: input streamed");

        let presence = seed_repo::tenant_presence(db, tenant).await?;
        if let Err(msg) = input_guards(rows.len(), presence, tenant, force) {
            return Err(SeedRunError::Guard(msg));
        }

        seed_from_rows(
            rows,
            &store,
            tenant,
            SYSTEM_AUTHOR,
            roster.as_ref(),
            Uuid::now_v7,
        )
        .await
        .map_err(SeedRunError::Failed)
    };

    tokio::time::timeout(SEED_TIMEOUT, run)
        .await
        .unwrap_or_else(|_| {
            Err(SeedRunError::Failed(anyhow::anyhow!(
                "persons-seed timed out after {}s",
                SEED_TIMEOUT.as_secs()
            )))
        })
}

pub(crate) fn resolve_tenant(
    configured: &str,
    distinct_in_persons: &[Uuid],
) -> Result<Uuid, String> {
    let configured = configured.trim();
    if !configured.is_empty() {
        return Uuid::parse_str(configured).map_err(|e| format!("invalid tenant_default_id: {e}"));
    }
    match distinct_in_persons {
        [sole] => Ok(*sole),
        [] => Err(
            "`gears.identity-resolution.config.tenant_default_id` is required for seed: the \
             persons log is empty, so there is no tenant to infer (fresh install — configure \
             the tenant explicitly)"
                .to_owned(),
        ),
        _ => Err(
            "`gears.identity-resolution.config.tenant_default_id` is required for seed: the \
             persons log holds several tenants, so inference is ambiguous — configure the \
             tenant explicitly"
                .to_owned(),
        ),
    }
}

fn input_guards(
    input_rows: usize,
    presence: seed_repo::TenantPresence,
    tenant: Uuid,
    force: bool,
) -> Result<(), String> {
    if force {
        return Ok(());
    }
    if input_rows == 0 {
        return Err(
            "input guard: identity_inputs returned 0 rows — the ingestion pipeline looks \
             broken or misconfigured (wrong ClickHouse URL/database?); re-run with --force \
             to seed anyway"
                .to_owned(),
        );
    }
    if !presence.has_own && presence.has_other {
        return Err(format!(
            "tenant guard: persons already holds rows under other tenant(s) and none under \
             the configured tenant {tenant} — seeding would mint a parallel person set under \
             a wrong tenant; fix tenant_default_id or re-run with --force"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::seed_repo::TenantPresence;

    fn tenant() -> Uuid {
        Uuid::from_u128(9)
    }

    fn presence(has_own: bool, has_other: bool) -> TenantPresence {
        TenantPresence { has_own, has_other }
    }

    #[test]
    fn empty_input_refused_and_names_the_table() -> anyhow::Result<()> {
        let Err(err) = input_guards(0, presence(false, false), tenant(), false) else {
            anyhow::bail!("empty input must be refused");
        };
        assert!(err.contains("identity_inputs"), "{err}");
        Ok(())
    }

    #[test]
    fn wrong_tenant_refused_and_names_the_tenant() -> anyhow::Result<()> {
        let Err(err) = input_guards(10, presence(false, true), tenant(), false) else {
            anyhow::bail!("wrong-tenant run must be refused");
        };
        assert!(err.contains("tenant"), "{err}");
        assert!(err.contains(&tenant().to_string()), "{err}");
        Ok(())
    }

    #[test]
    fn force_overrides_both_guards() {
        assert!(input_guards(0, presence(false, false), tenant(), true).is_ok());
        assert!(input_guards(10, presence(false, true), tenant(), true).is_ok());
    }

    #[test]
    fn fresh_install_passes_unforced() {
        assert!(input_guards(10, presence(false, false), tenant(), false).is_ok());
    }

    #[test]
    fn steady_state_passes_unforced() {
        assert!(input_guards(10, presence(true, true), tenant(), false).is_ok());
    }

    #[tokio::test]
    async fn unsupported_mode_fails_before_any_connect() {
        let cfg = crate::config::GearConfig::default();
        let err = run(&cfg, "no-such-mode", false).await;
        assert!(matches!(err, Err(SeedRunError::Failed(_))));
    }

    #[tokio::test]
    async fn default_config_fails_cleanly() {
        let cfg = crate::config::GearConfig::default();
        let err = run(&cfg, LINK_BY_EMAIL_MODE, false).await;
        assert!(matches!(err, Err(SeedRunError::Failed(_))));
    }

    #[test]
    fn configured_tenant_wins_over_inference() -> anyhow::Result<()> {
        let configured = tenant();
        let resolved = resolve_tenant(
            &configured.to_string(),
            &[Uuid::from_u128(1), Uuid::from_u128(2)],
        )
        .map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(resolved, configured);
        Ok(())
    }

    #[test]
    fn invalid_configured_tenant_is_refused() {
        assert!(resolve_tenant("not-a-uuid", &[]).is_err());
    }

    #[test]
    fn empty_config_infers_the_sole_tenant() -> anyhow::Result<()> {
        let sole = tenant();
        let resolved = resolve_tenant("  ", &[sole]).map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(resolved, sole);
        Ok(())
    }

    #[test]
    fn empty_config_with_no_tenants_is_refused_as_fresh_install() -> anyhow::Result<()> {
        let Err(msg) = resolve_tenant("", &[]) else {
            anyhow::bail!("empty persons log must not infer a tenant");
        };
        assert!(msg.contains("fresh install"), "{msg}");
        Ok(())
    }

    #[test]
    fn empty_config_with_several_tenants_is_refused_as_ambiguous() -> anyhow::Result<()> {
        let Err(msg) = resolve_tenant("", &[Uuid::from_u128(1), Uuid::from_u128(2)]) else {
            anyhow::bail!("several tenants must not infer");
        };
        assert!(msg.contains("ambiguous"), "{msg}");
        Ok(())
    }

    #[test]
    fn the_roster_comes_from_its_own_configuration_field() {
        let config = GearConfig {
            roster_source_type: "bamboohr".to_owned(),
            org_chart_source_type: "ms-entra".to_owned(),
            ..GearConfig::default()
        };

        let named = roster_source(&config);

        assert_eq!(named.as_ref().map(RosterSource::name), Some("bamboohr"));
        assert!(
            roster_source(&GearConfig::default()).is_none(),
            "the default configuration names no roster"
        );
    }
}
