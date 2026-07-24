//! Insight Identity Resolution service (Rust port of the .NET `identity` service,
//! epic #1602).
//!
//! Boots as a gears-rust host on [`toolkit::bootstrap::run_server`] — same host
//! pattern as `services/analytics`. Auth is ENABLED (`NGINX_BFF` R1): the
//! `oidc-authn-plugin` verifies the gateway JWT and maps its claims into the
//! request `SecurityContext`. Implements the full ported surface: `POST
//! /v1/profiles`, persons-seed, roles / person-roles / visibility, org subchart,
//! and the internal service-only by-email lookup.

mod api;
mod config;
mod domain;
mod gear;
mod infra;

// System gears — linked via inventory for the REST host and the gateway-JWT auth
// pipeline. `use … as _;` is load-bearing: the gears register through `inventory`
// at link time, so an unreferenced crate is dropped and never registers. Same set
// as the analytics host (incl. `oidc-authn-plugin`, which enforces auth).
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
use clap::Parser;
use toolkit::bootstrap::{AppConfig, run_server};

/// Identity Resolution service.
#[derive(Parser)]
#[command(name = "identity-resolution")]
#[command(about = "Insight Identity Resolution service (Rust port of .NET identity)")]
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
