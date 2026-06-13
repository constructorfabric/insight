//! Insight API Gateway
//!
//! Server binary that bootstraps the cyberfabric runtime with:
//! - API Gateway (HTTP server, `OpenAPI`, CORS, rate limiting, auth middleware)
//! - OIDC `AuthN` plugin (JWT validation against Okta/Keycloak/Auth0)
//! - `AuthZ` resolver (static plugin for now, custom org-tree plugin later)
//! - Tenant resolver (single-tenant plugin)
//!
//! # Configuration
//!
//! See `config/insight.yaml` for production config and `config/no-auth.yaml` for local dev.
//!
//! Usage:
//!   insight-api-gateway run -c config/insight.yaml
//!   insight-api-gateway check -c config/insight.yaml

// Insight gears (compiled into the binary, registered via inventory).
mod auth_info;
mod proxy;

// Link external modules via inventory — runtime discovers them automatically.
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
use toolkit::bootstrap::{AppConfig, run_migrate, run_server};

/// Insight API Gateway — entry point for all backend services.
#[derive(Parser)]
#[command(name = "insight-api-gateway")]
#[command(about = "Insight platform API Gateway with OIDC authentication")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Path to YAML configuration file.
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Print effective configuration and exit.
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
    /// Start the API gateway server (default).
    Run,
    /// Validate configuration and exit.
    Check,
    /// Run database migrations and exit.
    Migrate,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Layered config: defaults → YAML → env (APP__*) → CLI overrides
    let mut config = AppConfig::load_or_default(cli.config.as_ref())?;
    config.apply_cli_overrides(cli.verbose);

    if cli.print_config {
        println!("Effective configuration:\n{}", config.to_yaml()?);
        return Ok(());
    }

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run_server(config).await,
        Commands::Migrate => run_migrate(config).await,
        Commands::Check => Ok(()),
    }
}
