//! CLI persons-seed runner — the engine behind the `seed` subcommand
//! (issue #1690: the org tree froze because nothing re-ran the seed).
//!
//! The seed is CLI-only: a Helm `CronJob` (and manual `kubectl create job`)
//! runs `identity-resolution seed` inside the cluster — no HTTP trigger, no
//! auth. One run is: advisory lock → zombie sweep → `operations` journal row
//! → input read → guards → the same domain pipeline the removed
//! `POST /v1/persons-seed` used → journal completed/failed.
//!
//! Concurrency: runs serialize on a per-tenant MariaDB `GET_LOCK` owned by an
//! RAII guard for the whole run (see `infra::db::SeedLockGuard` — every exit
//! path releases it) — covers cron-vs-manual overlap and
//! multiple Insight instances sharing one database. A concurrent run fails
//! fast ([`SeedRunError::LockBusy`], exit code 2) instead of queueing a stale
//! re-run behind the active one.
//!
//! Guards (both overridable with `--force`, both recorded as a `failed`
//! operation so the journal explains why nothing was written):
//!   * empty input — 0 `identity_inputs` rows means a broken/misconfigured
//!     pipeline (wrong ClickHouse URL/database, wiped stand), not "no people";
//!   * wrong tenant — `persons` rows exist under OTHER tenants and none under
//!     the configured one: seeding would mint a parallel person universe
//!     under the wrong tenant (the HOTFIX(#1550) failure mode), which the append-only
//!     log cannot undo. A genuinely fresh install (empty `persons`) passes.

use std::time::Duration;

use uuid::Uuid;

use crate::config::GearConfig;
use crate::domain::roster::RosterSource;
use crate::domain::seed_service::{IdentityInputsReader, SeedSummary, seed_from_rows};
use crate::infra::db::{self, ops_repo, seed_repo};
use crate::infra::identity_inputs::ClickHouseIdentityInputsReader;
use crate::infra::metrics;

/// Author stamped on CLI-run operations and seed-minted observation rows.
/// Continues the established convention: the retired Python seed stamped
/// system-written rows with the nil UUID, so existing installs already carry
/// it. No FK constrains `author_person_id`, and the API-layer nil-caller
/// check (`api::gate`) inspects the JWT subject, not data rows.
pub const SYSTEM_AUTHOR: Uuid = Uuid::nil();

/// The only seed mode the pipeline implements.
pub const LINK_BY_EMAIL_MODE: &str = "link-by-email";

/// Upper bound on the read + pipeline — same ceiling the removed queue
/// worker used; a hung ClickHouse/MariaDB call fails the Job (with the
/// journal row updated to `failed`) instead of wedging it past the
/// `CronJob`'s next tick (the advisory lock would hold that tick off).
const SEED_TIMEOUT: Duration = Duration::from_mins(10);

/// Backstop over the WHOLE lock-held critical section — the zombie sweep and
/// the journal writes are MariaDB calls outside [`SEED_TIMEOUT`]'s scope, and
/// a hang in any of them would hold the advisory lock just as effectively as
/// a hung pipeline. A run cut off by THIS timeout may leave its `operations`
/// row `running`; the next run's zombie sweep reclaims it. The chart's
/// `activeDeadlineSeconds` (900s) stays the final out-of-process backstop.
const RUN_TIMEOUT: Duration = Duration::from_mins(12);

/// How long a run waits for the per-tenant lock before reporting `LockBusy`.
/// Unlike the publish lock, a busy seed lock never means "covered": the
/// holder read `identity_inputs` at its own start, so a pipeline seed that
/// gave up here would let gold build over identities its sync just landed.
const LOCK_WAIT_SECS: u32 = 15;

/// How stale a `queued`/`running` operation must be before the pre-run sweep
/// reclaims it. A killed Job pod leaves its row `running` forever otherwise —
/// the in-process state is gone, only the next run can clean up.
const ZOMBIE_CUTOFF_HOURS: i64 = 1;

/// Why a seed run did not complete — the `seed` subcommand maps each variant
/// to a distinct exit code so a `CronJob` failure is diagnosable from the Job
/// status alone (0 ok / 1 failed / 2 lock busy / 3 guard).
#[derive(Debug)]
pub enum SeedRunError {
    /// Another run holds the per-tenant advisory lock.
    LockBusy,
    /// An input guard refused the run (message is operator-facing and is
    /// persisted verbatim as the operation's `error_message`).
    Guard(String),
    /// The run itself failed (connect, read, pipeline, or journal write).
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for SeedRunError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed(e)
    }
}

/// Run one CLI persons-seed end to end. See the module docs for the shape.
///
/// # Errors
///
/// [`SeedRunError`] — lock busy, guard refusal, or a failed run.
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
    // SINGLE-TENANT BY DESIGN, gated on HOTFIX(#1550): one run seeds exactly
    // one tenant — the configured one. A true multi-tenant mode ("enumerate
    // tenants, seed each") is NOT possible yet: the identity_inputs reader
    // deliberately reads the WHOLE table with no tenant filter (see the
    // HOTFIX(#1550) block in `infra::identity_inputs` — the dbt producer
    // hashes tenant ids, so there is nothing to filter on), which means every
    // tenant's run would ingest every other tenant's rows and mint duplicate
    // person universes. Once the producer writes real tenant UUIDs and the
    // reader filter returns, this becomes a loop over tenants — the runner is
    // already per-tenant everywhere below (lock name, journal row, writes).
    let db = db::connect(&config.database_url).await?;
    // Same boot gate as the server: a seed against a schema behind this build
    // would fail per-query mid-run instead of upfront.
    db::assert_schema_compatible(&db).await?;

    // Tenant resolution: the configured tenant_default_id wins; when it is
    // EMPTY (existing installs whose pre-created config Secret predates the
    // seed and cannot be touched right now — dev is one), fall back to the
    // single tenant the persons log already holds. Inference is deliberately
    // narrow: exactly ONE distinct tenant is the only unambiguous case —
    // writing under the sole tenant the data already lives under is what an
    // operator would configure anyway. Zero (fresh install) or several
    // tenants → refuse and demand explicit config; guessing there recreates
    // the HOTFIX(#1550) wrong-tenant hazard the guards exist to prevent.
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
    // RAII: the guard owns the lock's dedicated session — every exit path
    // (return, cancellation, crash) releases the lock, see `SeedLockGuard`.
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

/// Everything that happens while the advisory lock is held: zombie sweep,
/// journal row, guarded seed, journal resolution.
async fn run_locked(
    db: &sea_orm::DatabaseConnection,
    config: &GearConfig,
    tenant: Uuid,
    mode: &str,
    force: bool,
) -> Result<SeedSummary, SeedRunError> {
    // Reclaim rows a killed run left behind. Log-only failure: a broken sweep
    // must not block the seed itself.
    let cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::hours(ZOMBIE_CUTOFF_HOURS);
    match ops_repo::sweep_zombies(db, cutoff).await {
        Ok(n) if n > 0 => tracing::warn!(swept = n, "persons-seed: reclaimed zombie operations"),
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "persons-seed: zombie sweep failed"),
    }

    // Journal row first, so every later failure (guard included) is recorded
    // and visible over the GET /v1/persons-seed endpoints.
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
            // Deliberate operator-facing text — safe to persist verbatim.
            tracing::warn!(%operation_id, %msg, "persons-seed: refused by input guard");
            if let Err(e) = ops_repo::fail(db, operation_id, &msg).await {
                tracing::error!(error = %e, %operation_id, "fail update failed");
            }
            Err(SeedRunError::Guard(msg))
        }
        Err(e) => {
            // Persist only a generic message: `error_message` is returned
            // verbatim by the GET endpoints, so raw driver/anyhow text must
            // not leak to callers (same rule the queue worker followed).
            if let Err(e2) =
                ops_repo::fail(db, operation_id, "persons-seed failed; see job logs").await
            {
                tracing::error!(error = %e2, %operation_id, "fail update failed");
            }
            Err(e)
        }
    }
}

/// The roster the configuration names, if any.
///
/// Split out to be testable: it sits beside `org_chart_source_type`, which is
/// also a single source type, and reading the wrong one of the two has no
/// symptom at all — the run succeeds and mints nothing.
fn roster_source(config: &GearConfig) -> Option<RosterSource> {
    RosterSource::parse(&config.roster_source_type)
}

/// Input read → guards → pipeline, bounded by [`SEED_TIMEOUT`].
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

/// The pure tenant-resolution decision (split out for unit tests): an
/// explicitly configured tenant always wins; an empty config falls back to
/// the SOLE tenant present in the persons log; anything ambiguous refuses
/// with an operator-facing message. `pub(crate)`: the sync runner journals
/// its runs under the same resolved tenant (its GET journal routes are
/// tenant-scoped, so a made-up tenant would hide the rows from admins).
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

/// The pure guard decision (see the module docs): refuse an empty input and
/// refuse a wrong-tenant run; `--force` overrides both. The returned message
/// is operator-facing — it lands verbatim in the operation's `error_message`
/// and the Job log.
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
        // Non-empty input, persons entirely empty — the bootstrap shape.
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
            // Deliberately different: this field is the neighbour a mistyped
            // read would land on, and both hold a bare source type.
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
