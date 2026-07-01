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
