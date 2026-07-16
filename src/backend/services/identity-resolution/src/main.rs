//! Insight Identity Resolution service (Rust port of the .NET `identity` service).
//!
//! Iteration-1 scaffold: boots as a gears-rust host on
//! [`toolkit::bootstrap::run_server`]. The `api-gateway` system gear is the REST
//! host (auth disabled — the platform gateway authenticates and proxies here).
//! There is no domain gear yet: this milestone only proves the host boots and
//! serves `/health`. The read endpoints (`GET /v1/persons/{email}`,
//! `POST /v1/profiles`) land in the next steps.

mod api;
mod auth;
mod config;
mod domain;
mod gear;
mod infra;

// System gears — linked via inventory for the REST host and the (disabled) auth
// pipeline. Same no-auth set as the analytics host.
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
use clap::Parser;
use toolkit::bootstrap::{AppConfig, run_server};

/// Identity Resolution service.
#[derive(Parser)]
#[command(name = "identity-resolution")]
#[command(about = "Insight Identity Resolution — read API (Rust port)")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Path to YAML configuration file.
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // Layered config: defaults -> YAML -> env (APP__*). Logging/OTel are
    // initialized by the bootstrap runtime, not here.
    let config = AppConfig::load_or_default(cli.config.as_ref())?;
    run_server(config).await
}
