//! The identity-resolution gear.
//!
//! Runs on the `api-gateway` system gear (the REST host) under
//! `toolkit::bootstrap::run_server`. [`IdentityResolutionGear::init`] builds the
//! runtime (MariaDB pool + persons-seed worker); [`register_rest`] mounts the
//! profile-read and persons-seed routes on the host router.
//!
//! [`register_rest`]: IdentityResolutionGear::register_rest

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use toolkit::api::OpenApiRegistry;
use toolkit::{Gear, GearCtx, RestApiCapability};

use crate::api::AppState;
use crate::config::GearConfig;

/// Identity-resolution gear. Capability: `rest` (HTTP surface). Config key is
/// the gear name `identity-resolution`; env overrides are
/// `APP__gears__identity-resolution__config__*`.
#[derive(Default)]
#[toolkit::gear(name = "identity-resolution", capabilities = [rest])]
pub struct IdentityResolutionGear {
    state: OnceLock<Arc<AppState>>,
}

#[async_trait]
impl Gear for IdentityResolutionGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let config: GearConfig = ctx.config()?;
        tracing::info!("starting identity-resolution gear");

        // Self-managed MariaDB pool (same approach as the analytics gear).
        let db = crate::infra::db::connect(&config.database_url).await?;

        // Persons-seed background worker: drains a job queue and runs each seed.
        // A single spawned task (like the analytics validators) owns the queue.
        // Capacity matches the .NET `PersonsSeedQueue` bound (100).
        let (seed_tx, seed_rx) = tokio::sync::mpsc::channel(100);
        let worker_db = db.clone();
        let worker_config = config.clone();
        tokio::spawn(async move {
            crate::api::seed::run_worker(seed_rx, worker_db, worker_config).await;
        });

        let state = AppState {
            db,
            config,
            seed_tx,
        };
        self.state
            .set(Arc::new(state))
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;
        Ok(())
    }
}

/// Pull `gears.identity-resolution.config` out of the loaded `AppConfig` and
/// deserialize it into [`GearConfig`] — for subcommands that run outside the
/// gear lifecycle (same helper shape as the analytics service).
fn extract_gear_config(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<GearConfig> {
    let raw = app
        .gears
        .get("identity-resolution")
        .and_then(|v| v.get("config"))
        .ok_or_else(|| {
            anyhow::anyhow!("missing `gears.identity-resolution.config` section in configuration")
        })?;
    let cfg: GearConfig = serde_json::from_value(raw.clone())?;
    Ok(cfg)
}

/// `migrate` subcommand: apply pending schema migrations + the first-admin
/// bootstrap, then exit. Runs on a dedicated single-connection session (the
/// advisory lock in `run_migrations` is session-scoped).
///
/// # Errors
///
/// Returns an error when the config section or `database_url` is missing, the
/// migration lock cannot be acquired, or a migration/bootstrap step fails.
pub async fn run_migrate(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<()> {
    tracing::info!("running migrations");
    let cfg = extract_gear_config(app)?;
    anyhow::ensure!(
        !cfg.database_url.is_empty(),
        "`gears.identity-resolution.config.database_url` is required for migrate"
    );
    let db = crate::infra::db::connect_single(&cfg.database_url).await?;
    // Migrations AND the first-admin bootstrap run under one advisory lock —
    // see `run_migrations` on why the bootstrap must stay inside the critical
    // section (unconstrained INSERT … WHERE NOT EXISTS between replicas).
    crate::infra::db::run_migrations(&db, &cfg).await?;
    Ok(())
}

/// Author recorded against rows written by `init-seed` — there is no real
/// caller (the whole point is to run before any person/admin exists), so a
/// nil id marks the run as system-initiated rather than attributing it to a
/// fabricated person.
const INIT_SEED_SYSTEM_AUTHOR: uuid::Uuid = uuid::Uuid::nil();

/// `init-seed` subcommand (#1956): run the persons-seed pipeline directly,
/// bypassing `POST /v1/persons-seed`'s admin gate entirely — for the one
/// situation that gate makes unreachable through the product's own APIs: a
/// fresh tenant with zero rows in `persons`/`person_roles`, where no caller
/// can ever be admin.
///
/// Three things keep this from silently misfiring, all required after review:
///   * It takes the SAME [`crate::infra::db::MIGRATION_LOCK`] advisory lock as
///     `run_migrations`, on a single-connection session, and runs the
///     freshness check + seed under it — not a separate lock. A fresh install
///     may have the `migrate` initContainer (still creating tables or
///     applying `bootstrap_admin_person_id`) and a QA-triggered `init-seed`
///     running close together; a distinct lock would let `init-seed` hit a
///     partial schema or race the bootstrap insert. Sharing the lock also
///     serializes two `init-seed` invocations against each other, so they
///     can't both mint distinct `person_id`s for the same accounts.
///   * The freshness check looks at actual data — `persons` AND `person_roles`
///     both empty for the tenant — not "no *active* admin" (a tenant with a
///     revoked admin, or seeded persons but a locked-out admin, has still
///     been administered before and must go through the normal HTTP path).
///   * A seed that reads zero rows from `identity.identity_inputs` (bronze
///     synced but the dbt transform hasn't populated it yet) is rejected
///     rather than reported as a successful bootstrap — otherwise the CLI
///     exits 0 having created nothing, and login keeps 403ing.
///
/// # Errors
///
/// Returns an error when required config is missing, `tenant_default_id` does
/// not parse, the advisory lock cannot be acquired, the tenant already has
/// `persons` or `person_roles` rows (use `POST /v1/persons-seed` instead), the
/// seed read zero accounts from ClickHouse, or the seed itself fails.
pub async fn run_init_seed(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<()> {
    use crate::infra::db::{MIGRATION_LOCK, MIGRATION_LOCK_TIMEOUT_SECS};
    use sea_orm::{ConnectionTrait, DbBackend, Statement};

    let cfg = extract_gear_config(app)?;
    anyhow::ensure!(
        !cfg.database_url.is_empty(),
        "`gears.identity-resolution.config.database_url` is required for init-seed"
    );
    anyhow::ensure!(
        !cfg.tenant_default_id.is_empty(),
        "`gears.identity-resolution.config.tenant_default_id` is required for init-seed"
    );
    let tenant = uuid::Uuid::parse_str(cfg.tenant_default_id.trim())
        .map_err(|e| anyhow::anyhow!("invalid tenant_default_id: {e}"))?;

    // Single-connection session: GET_LOCK is session-scoped and must share the
    // session with the guard check and the seed writes it protects. Same lock
    // name as `run_migrations` — see the doc comment above on why.
    let db = crate::infra::db::connect_single(&cfg.database_url).await?;

    let acquired: Option<i8> = db
        .query_one(Statement::from_sql_and_values(
            DbBackend::MySql,
            "SELECT GET_LOCK(?, ?)",
            [MIGRATION_LOCK.into(), MIGRATION_LOCK_TIMEOUT_SECS.into()],
        ))
        .await?
        .map(|r| r.try_get_by_index::<Option<i8>>(0))
        .transpose()?
        .flatten();
    anyhow::ensure!(
        acquired == Some(1),
        "could not acquire the `{MIGRATION_LOCK}` advisory lock within \
         {MIGRATION_LOCK_TIMEOUT_SECS}s — is a migrate or another init-seed run stuck?"
    );

    let result = run_init_seed_locked(&db, &cfg, tenant).await;

    // Best-effort release either way; the lock also dies with the session.
    let _ = db
        .execute(Statement::from_sql_and_values(
            DbBackend::MySql,
            "SELECT RELEASE_LOCK(?)",
            [MIGRATION_LOCK.into()],
        ))
        .await;

    result
}

/// The guarded body of `init-seed`, run inside the shared `MIGRATION_LOCK`
/// critical section: freshness check, seed run, and non-empty-result
/// validation.
async fn run_init_seed_locked(
    db: &sea_orm::DatabaseConnection,
    cfg: &crate::config::GearConfig,
    tenant: uuid::Uuid,
) -> anyhow::Result<()> {
    let persons_count = crate::infra::db::persons_repo::count_for_tenant(db, tenant).await?;
    let roles_count = crate::infra::db::person_roles_repo::count_for_tenant(db, tenant).await?;
    anyhow::ensure!(
        persons_count == 0 && roles_count == 0,
        "tenant {tenant} is not fresh (persons={persons_count}, person_roles={roles_count}) — \
         refusing to init-seed; use POST /v1/persons-seed instead"
    );

    tracing::warn!(
        %tenant,
        "init-seed: running persons-seed with the admin gate bypassed \
         (tenant has no persons/person_roles yet) — local/QA bootstrap only, see #1956"
    );

    let reader = crate::infra::identity_inputs::ClickHouseIdentityInputsReader::connect(
        &cfg.clickhouse_url,
        &cfg.clickhouse_database,
        &cfg.clickhouse_user,
        &cfg.clickhouse_password,
    );
    let store = crate::infra::db::seed_repo::MariaDbSeedStore::new(db);

    let summary = crate::domain::seed_service::run_seed(
        &reader,
        &store,
        tenant,
        INIT_SEED_SYSTEM_AUTHOR,
        uuid::Uuid::now_v7,
    )
    .await?;

    // A seed that read nothing "succeeds" at rebuilding empty projections —
    // exactly the #1956 trap (CLI exits 0, login keeps 403ing). Treat it as a
    // failure so the operator investigates ingestion instead of assuming the
    // tenant is bootstrapped.
    anyhow::ensure!(
        summary.accounts_read > 0,
        "init-seed read 0 accounts from identity.identity_inputs for tenant {tenant} — \
         sync/transform the org-chart connector before running init-seed"
    );
    let persons_created = summary.reused_known + summary.linked_by_email + summary.minted;
    anyhow::ensure!(
        persons_created > 0,
        "init-seed read {} account(s) but resolved 0 persons for tenant {tenant} \
         (all closed / no email) — nothing to log in as",
        summary.accounts_read
    );

    tracing::info!(?summary, %tenant, "init-seed: persons-seed completed");
    Ok(())
}

impl RestApiCapability for IdentityResolutionGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: axum::Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        let state = self
            .state
            .get()
            .ok_or_else(|| anyhow::anyhow!("identity-resolution gear not initialized"))?
            .clone();
        Ok(crate::api::register_routes(router, openapi, state))
    }
}
