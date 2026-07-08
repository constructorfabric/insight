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
//! analytics --config config.yaml          # run the host
//! analytics --config config.yaml migrate  # run migrations + probes and exit
//! analytics openapi                        # print the OpenAPI document and exit
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
#[command(name = "analytics")]
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
    /// Print the OpenAPI document to stdout and exit. Built offline from the
    /// route table — no database, no HTTP listener, no config needed. Used to
    /// regenerate docs/components/backend/analytics/openapi.json and to
    /// drift-check it in CI.
    Openapi,
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
        // Validate the gear config (section present, deserializes, required
        // URLs set) without connecting to any backend.
        Commands::Check => gear::check_config(&config),
        // Emit the OpenAPI document offline (no backends) — see `print_openapi`.
        Commands::Openapi => print_openapi(),
    }
}

/// Print the analytics `OpenAPI` document as pretty JSON. Offline — see
/// [`api::openapi_document`]. No config or backends are touched, and no logging
/// subscriber is initialized on this path, so stdout stays pure JSON.
fn print_openapi() -> Result<()> {
    let doc = api::openapi_document()?;
    println!("{}", serde_json::to_string_pretty(&doc)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The `openapi` subcommand's happy path: build the document offline and
    /// write it to stdout (captured by the harness).
    #[test]
    fn print_openapi_writes_the_document() -> anyhow::Result<()> {
        super::print_openapi()
    }
}
