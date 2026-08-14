//! Binary entry point. Boots as a gears-rust host on
//! [`toolkit::bootstrap::run_server`] — same host pattern as
//! `services/identity-resolution`, but the service hosts its own routes (see
//! `gear.rs`) and has no gateway-JWT auth pipeline: it is Airbyte-only and
//! cluster-internal (never behind the platform gateway, never called by other
//! services or users directly), and `/v1` is guarded by a static bearer token
//! from gear config.

// System gears — linked via inventory. `use … as _;` is load-bearing: the
// gears register through `inventory` at link time, so an unreferenced crate is
// dropped and never registers. Deliberately minimal set (no api-gateway host,
// no authn / authz / tenant plugins — own bearer auth instead). The service's
// own gear registers the same way from the lib crate.
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
    /// Print the `OpenAPI` document as JSON and exit.
    Openapi,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();
    let command = cli.command.take().unwrap_or(Commands::Run);

    // Loaded per command rather than up front, so the offline `openapi` emit
    // cannot fail on a config file it never reads.
    let load_config = || AppConfig::load_or_default(cli.config.as_ref());

    match command {
        Commands::Run => run_server(load_config()?).await,
        Commands::Openapi => print_openapi(),
    }
}

/// Offline emit — no config, no listener. No logging subscriber is installed
/// on this path, so stdout stays pure JSON for the drift gate to consume.
fn print_openapi() -> Result<()> {
    let doc = git_cli_proxy::api::openapi_document()?;
    println!("{}", serde_json::to_string_pretty(&doc)?);
    Ok(())
}
