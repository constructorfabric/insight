//! Insight Previews service (epic #1981, phase 1 — #2372).
//!
//! Boots as a gears-rust host on [`toolkit::bootstrap::run_server`] — same
//! host pattern as `services/identity-resolution`. Auth is ENABLED: the
//! `oidc-authn-plugin` verifies the gateway JWT and maps its claims into the
//! request `SecurityContext`. Manages preview experiments: each is one FE
//! build served under `/exp/<name>`, stored as a Deployment/Service/HTTPRoute
//! trio in a dedicated namespace — Kubernetes is the only store.

mod api;
mod config;
mod domain;
mod gear;
mod infra;
mod sweep;

// System gears — linked via inventory for the REST host and the gateway-JWT
// auth pipeline. `use … as _;` is load-bearing: the gears register through
// `inventory` at link time, so an unreferenced crate is dropped and never
// registers. Same set as the identity-resolution host.
use api_gateway as _;
use authn_resolver as _;
use authz_resolver as _;
use gear_orchestrator as _;
use grpc_hub as _;
use oidc_authn_plugin as _;
use single_tenant_tr_plugin as _;
use static_authz_plugin as _;
use tenant_resolver as _;
use types_registry as _;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use toolkit::bootstrap::{AppConfig, run_server};

/// Previews service.
#[derive(Parser)]
#[command(name = "previews")]
#[command(about = "Insight Previews service")]
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
    /// Print the OpenAPI document to stdout and exit. Offline — no config,
    /// no cluster, no logging subscriber, so stdout stays pure JSON. Backs
    /// the committed-doc drift gate.
    Openapi,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();
    let command = cli.command.take().unwrap_or(Commands::Run);

    match command {
        Commands::Openapi => print_openapi(),
        Commands::Run => run_server(AppConfig::load_or_default(cli.config.as_ref())?).await,
    }
}

/// Print the `OpenAPI` document as pretty JSON. Offline — see
/// [`api::openapi_document`]. No logging subscriber is installed on this
/// path, so stdout stays pure JSON for the drift gate to consume.
fn print_openapi() -> Result<()> {
    let doc = api::openapi_document()?;
    println!("{}", serde_json::to_string_pretty(&doc)?);
    Ok(())
}
