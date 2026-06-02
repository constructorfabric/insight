//! Analytics API — read-only query service over predefined `ClickHouse` metrics.
//!
//! Serves admin-defined metrics (SQL queries stored in `MariaDB`) with tenant-scoped,
//! org-scoped security filters and `OData`-style querying.
//!
//! # Usage
//!
//! ```text
//! analytics-api --config config.yaml
//! analytics-api --config config.yaml migrate
//! ```

mod api;
mod auth;
mod config;
mod domain;
mod infra;
mod migration;

use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use crate::domain::admin_threshold::AdminThresholdService;
use crate::domain::auth::ConfigTenantAuthorization;
use crate::domain::catalog::{CatalogReader, ThresholdResolver};
use crate::infra::cache::catalog_cache::{CatalogCache, NoopCatalogCache, RedisCatalogCache};

/// Analytics API service.
#[derive(Parser)]
#[command(name = "analytics-api")]
#[command(about = "Insight Analytics API — query service over `ClickHouse` metrics")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Path to YAML configuration file.
    #[arg(short, long)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the server (default).
    Run,
    /// Run database migrations and exit.
    Migrate,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let cli = Cli::parse();

    let cfg = config::AppConfig::load(cli.config.as_deref())?;

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run_server(cfg).await,
        Commands::Migrate => run_migrate(cfg).await,
    }
}

async fn run_server(cfg: config::AppConfig) -> anyhow::Result<()> {
    tracing::info!("starting analytics-api");

    // Connect to MariaDB
    let db = infra::db::connect(&cfg.database_url).await?;

    // Run pending migrations
    infra::db::run_migrations(&db).await?;

    // Refuse to start if any required CHECK constraint is missing. Our
    // bitnami-shipped MariaDB is 11.x, but on customer-managed DBs (BYO-DB
    // installs, RDS, Cloud SQL, on-prem) we can't audit the version or
    // `sql_mode`. See `infra/db/check_probe` and DESIGN §2.2
    // `cpt-metric-cat-constraint-mariadb-check` for the full rationale.
    infra::db::check_probe::assert_required_checks(&db).await?;

    // Refuse to start if any enabled `metric_catalog` row is missing its
    // `product-default` `metric_threshold` floor (Refs #523). The resolver
    // walks down to product-default; a missing floor would make the
    // catalog read return no threshold and break the byte-for-byte
    // bullet-rendering gate. See `infra/db/product_default_probe` and
    // DESIGN §3.6 + `cpt-metric-cat-fr-tenant-thresholds`.
    infra::db::product_default_probe::assert_product_default_present(&db).await?;

    // Catalog cache (Refs #524). Use Redis when `redis_url` is configured;
    // otherwise fall back to the no-op stub for single-replica dev installs.
    // The Redis-mode boot path is best-effort — if Redis is unreachable at
    // startup, we degrade to the no-op rather than refusing to serve. The
    // alternative ("refuse to boot until Redis is up") would make a Redis
    // outage gate every analytics-api deploy, which is precisely the kind
    // of upstream-availability coupling DESIGN §3.5 keeps swappable.
    let catalog_cache: Arc<dyn CatalogCache> = if cfg.redis_url.is_empty() {
        tracing::info!(
            "catalog_cache: redis_url not configured; using no-op stub. \
             Multi-replica deploys MUST configure redis_url per \
             cpt-metric-cat-nfr-cross-replica-invalidation."
        );
        Arc::new(NoopCatalogCache::default())
    } else {
        match RedisCatalogCache::connect(&cfg.redis_url).await {
            Ok(c) => {
                tracing::info!(
                    redis_url = %cfg.redis_url,
                    "catalog_cache: Redis backend connected"
                );
                Arc::new(c)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    redis_url = %cfg.redis_url,
                    "catalog_cache: Redis connection failed at boot; \
                     degrading to no-op stub. Cross-replica invalidation \
                     NFR will not hold until Redis is restored."
                );
                Arc::new(NoopCatalogCache::default())
            }
        }
    };

    // Flush the catalog cache so newly seeded rows are visible on the
    // next `POST /v1/catalog/get_metrics` read without waiting for the TTL.
    // Best-effort — a Redis blip MUST NOT gate service boot.
    if let Err(e) = catalog_cache.flush_all().await {
        tracing::warn!(error = %e, "catalog_cache: flush_all failed at boot; continuing");
    }

    // Threshold resolver + reader (Refs #524).
    let catalog_reader =
        CatalogReader::new(catalog_cache.clone(), ThresholdResolver::new(db.clone()));

    // Admin-CRUD service (Refs #525). Composes the lock-enforcer +
    // audit-emitter + cache + schema-validator behind the validation
    // gauntlet documented at `domain::admin_threshold::service`.
    // Constructor is moved below `validator` + `tenant_auth` because it
    // borrows both — see further down.

    // Connect to ClickHouse
    let mut ch_config =
        insight_clickhouse::Config::new(&cfg.clickhouse_url, &cfg.clickhouse_database);
    if let (Some(user), Some(password)) = (&cfg.clickhouse_user, &cfg.clickhouse_password) {
        ch_config = ch_config.with_auth(user, password);
    }
    let ch = insight_clickhouse::Client::new(ch_config);

    // Identity client
    let identity = infra::identity::IdentityClient::new(&cfg.identity_url);

    // Build the schema-validator (Refs #521). The validator is held in
    // AppState (so admin-crud can call its per-write hook from #525) and
    // also cloned into a post-readiness background task that runs the
    // startup pass.
    let validator = domain::schema_validator::SchemaValidator::new(db.clone(), ch.clone());

    // Catalog auth-trait (Refs #522 / #525). v1 stub: `is_tenant_admin`
    // returns true unconditionally — see `domain::auth` module doc-comment.
    // Production deployment MUST swap this for the real-Auth implementation.
    let tenant_auth: Arc<dyn crate::domain::auth::TenantAuthorization> = Arc::new(
        ConfigTenantAuthorization::new(cfg.metric_catalog.tenant_default_id),
    );

    // Admin-CRUD service (Refs #525).
    let admin_threshold = AdminThresholdService::new(
        db.clone(),
        tenant_auth.clone(),
        catalog_cache.clone(),
        validator.clone(),
    );

    // Build app state
    let state = api::AppState {
        db,
        ch,
        identity,
        config: cfg.clone(),
        validator: validator.clone(),
        tenant_auth,
        catalog_reader,
        admin_threshold,
    };

    // Build router
    let app = api::router(state);

    // Start server. The HTTP listener binds first — `/health` returns 200
    // unconditionally, so readiness is satisfied before the validator's
    // first ClickHouse call. This is the load-bearing "post-readiness"
    // requirement of `cpt-metric-cat-component-schema-validator`: a
    // ClickHouse outage at boot must NOT block deploys or restart-storm
    // the service.
    let addr = cfg.bind_addr.parse::<std::net::SocketAddr>()?;
    tracing::info!(addr = %addr, "listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tokio::spawn(async move {
        validator.validate_all().await;
    });

    axum::serve(listener, app).await?;

    Ok(())
}

async fn run_migrate(cfg: config::AppConfig) -> anyhow::Result<()> {
    tracing::info!("running migrations");
    let db = infra::db::connect(&cfg.database_url).await?;
    infra::db::run_migrations(&db).await?;

    // Same probe as `run_server`. An operator running `analytics-api migrate`
    // after a schema rollback wants the integrity signal too — the typical
    // recovery path is `migrate` first, restart the service second, and
    // dropping the probe here would silently re-greenlight a DB that's
    // missing a CHECK the application relies on.
    infra::db::check_probe::assert_required_checks(&db).await?;

    // Same rationale for the product-default probe: an operator running
    // `migrate` standalone wants to know immediately if the seed left the
    // catalog with orphaned enabled rows.
    infra::db::product_default_probe::assert_product_default_present(&db).await?;

    // DESIGN §3.6's seed-migration sequence ends with
    // `cache_layer.flush_all() → ack`. Operators who run `analytics-api
    // migrate` as a standalone step (e.g., a one-shot Kubernetes Job, a
    // post-deploy hook) need the same flush — otherwise the seed lands
    // and the cache stays stale until something triggers a server boot.
    // Best-effort per the same rationale as `run_server` — never block
    // migrate on a Redis blip.
    let catalog_cache: Arc<dyn CatalogCache> = if cfg.redis_url.is_empty() {
        Arc::new(NoopCatalogCache::default())
    } else {
        match RedisCatalogCache::connect(&cfg.redis_url).await {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "catalog_cache: Redis connection failed during migrate; \
                     skipping flush (next server boot will retry)"
                );
                Arc::new(NoopCatalogCache::default())
            }
        }
    };
    if let Err(e) = catalog_cache.flush_all().await {
        tracing::warn!(error = %e, "catalog_cache: flush_all failed after migrate; continuing");
    }

    tracing::info!("migrations complete");
    Ok(())
}
