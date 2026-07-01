//! Analytics API — read-only query service over predefined `ClickHouse` metrics.
//!
//! Serves admin-defined metrics (SQL queries stored in `MariaDB`) with tenant-scoped,
//! org-scoped security filters and `OData`-style querying.
//!
//! # Architecture
//!
//! Runs as a gears-rust host on [`toolkit::bootstrap::run_server`]. The
//! `api-gateway` system gear is the REST host; the analytics functionality is
//! the [`gear::AnalyticsApiGear`] (`rest` + `stateful`). Auth is **disabled** on
//! this host — the platform api-gateway is the sole authenticator and proxies
//! to us — so the host injects a single-tenant `SecurityContext` and a thin
//! layer overrides the tenant from `X-Insight-Tenant-Id` (see [`crate::auth`]).
//!
//! The MariaDB connection, its 45 sea-orm migrations, and the startup CHECK /
//! product-default probes remain self-managed inside the gear (we do not use
//! the toolkit `db` capability — `ClickHouse` is not a toolkit-db backend
//! anyway).
//!
//! # Usage
//!
//! ```text
//! analytics-api --config config.yaml          # run the host
//! analytics-api --config config.yaml migrate  # run migrations + probes and exit
//! ```

mod api;
mod auth;
mod config;
mod domain;
mod gear;
mod infra;
mod migration;

// System gears — linked via inventory for the REST host and the (disabled)
// auth pipeline. Mirrors the api-gateway service's no-auth gear set, minus the
// OIDC plugin / proxy / auth-info (this host never authenticates).
use api_gateway as _;
use authn_resolver as _;
use authz_resolver as _;
use gear_orchestrator as _;
use grpc_hub as _;
use single_tenant_tr_plugin as _;
use static_authz_plugin as _;
use tenant_resolver as _;
use types_registry as _;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use toolkit::bootstrap::{AppConfig, run_server};

/// Analytics API service.
#[derive(Parser)]
#[command(name = "analytics-api")]
#[command(about = "Insight Analytics API — query service over `ClickHouse` metrics")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Path to YAML configuration file.
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Print the effective configuration and exit.
    #[arg(long)]
    print_config: bool,

    /// Increase log verbosity (-v = debug, -vv = trace).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the server (default).
    Run,
    /// Run database migrations + startup probes and exit.
    Migrate,
    /// Validate configuration and exit.
    Check,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Layered config: defaults -> YAML -> env (APP__*) -> CLI overrides.
    // Logging/OTel are initialized by the bootstrap runtime, not here.
    let mut config = AppConfig::load_or_default(cli.config.as_ref())?;
    config.apply_cli_overrides(cli.verbose);
    fold_gear_env_alias(&mut config);

    if cli.print_config {
        println!("Effective configuration:\n{}", config.to_yaml()?);
        return Ok(());
    }

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run_server(config).await,
        Commands::Migrate => gear::run_migrate(&config).await,
        Commands::Check => Ok(()),
    }
}

/// Fold the identifier-safe `analytics_api` gears alias into the kebab-case
/// gear key `analytics-api`.
///
/// The gear name is macro-locked to kebab-case (`analytics-api`), so its config
/// lives under `gears.analytics-api`. But config overrides arrive as env vars,
/// and hyphenated names (`APP__gears__analytics-api__config__*`) are silently
/// dropped by the compose `sh`/dash entrypoint (dash discards env names that
/// aren't valid identifiers) and skipped by Kubernetes `envFrom`. Operators
/// therefore set the identifier-safe `APP__gears__analytics_api__config__*`,
/// which figment lands under a separate `gears.analytics_api` key. The toolkit
/// does no kebab/snake normalization, so we bridge it here: deep-merge the
/// `analytics_api` alias into `analytics-api` (alias values win on leaves), so
/// the overrides reach the gear regardless of how they were delivered.
fn fold_gear_env_alias(config: &mut AppConfig) {
    const ALIAS: &str = "analytics_api";
    const CANONICAL: &str = "analytics-api";

    let Some(alias) = config.gears.remove(ALIAS) else {
        return;
    };
    match config.gears.get_mut(CANONICAL) {
        Some(existing) => deep_merge(existing, alias),
        None => {
            config.gears.insert(CANONICAL.to_owned(), alias);
        }
    }
}

/// Recursively merge `overlay` into `base`; objects merge key-by-key, and any
/// non-object (leaf) in `overlay` replaces the value in `base`.
fn deep_merge(base: &mut serde_json::Value, overlay: serde_json::Value) {
    match (base, overlay) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                match base_map.get_mut(&key) {
                    Some(base_value) => deep_merge(base_value, value),
                    None => {
                        base_map.insert(key, value);
                    }
                }
            }
        }
        (base_slot, overlay_value) => *base_slot = overlay_value,
    }
}
