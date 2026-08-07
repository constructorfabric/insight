//! Binary entry point. Boots as a gears-rust host on
//! [`toolkit::bootstrap::run_server`] — same host pattern as
//! `services/identity-resolution`, but WITHOUT the gateway-JWT auth pipeline:
//! the service is Airbyte-only and cluster-internal (never behind the platform
//! gateway, never called by other services or users directly), the host runs
//! with `auth_disabled: true`, and `/v1` is guarded by a static bearer token
//! from gear config.

// System gears — linked via inventory for the REST host. `use … as _;` is
// load-bearing: the gears register through `inventory` at link time, so an
// unreferenced crate is dropped and never registers. Deliberately minimal set
// (no oidc-authn-plugin / authz / tenant plugins — own bearer auth instead).
// The service's own gear registers the same way from the lib crate.
use api_gateway as _;
use authn_resolver as _;
use gear_orchestrator as _;
use git_cli_proxy as _;
use grpc_hub as _;
use types_registry as _;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use toolkit::bootstrap::{AppConfig, run_server};

/// Git CLI Proxy service.
#[derive(Parser)]
#[command(name = "git-cli-proxy")]
#[command(about = "Insight Git CLI Proxy — clone-based git data extraction")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Path to YAML configuration file.
    #[arg(short, long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the server (default).
    Run,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = AppConfig::load_or_default(cli.config.as_ref())?;
    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run_server(config).await,
    }
}
