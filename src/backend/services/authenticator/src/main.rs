//! Authenticator — OIDC login, opaque sessions, and the cookie-to-JWT exchange
//! behind the nginx gateway (the BFF / token-handler pattern; NGINX_BFF.md §4.1,
//! §11).
//!
//! Runs as a gears-rust host on [`toolkit::bootstrap::run_server`]. The REST
//! host is the gears-rust toolkit `api-gateway` system gear
//! (`cf-gears-api-gateway`, the HTTP-server framework every gear runs on — NOT
//! the Insight platform api-gateway service that the nginx edge replaces); the
//! authenticator functionality is [`gear::AuthenticatorGear`] (`rest` +
//! `stateful`). Its `/auth/*` and `/internal/authz` endpoints are `.public()`
//! — the credential is the session cookie, checked in the handler.
//!
//! # Usage
//! ```text
//! authenticator -c config/insight.yaml run    # start the host
//! authenticator -c config/insight.yaml check  # validate config and exit
//! ```

// Doc comments reference spec artifacts (NGINX_BFF.md, DESIGN, RFC ids, header
// names) heavily; the pedantic doc_markdown lint would demand backticks around
// all of them, which hurts readability of the prose more than it helps.
#![allow(clippy::doc_markdown)]

mod api;
mod config;
mod cookie;
mod gear;
mod identity;
mod jwt;
mod local_client;
mod oidc;
mod service_token;
mod session;

// System gears — linked via inventory for the REST host + auth pipeline.
// Mirrors the analytics service's set (the authenticator authenticates its own
// admin surface with the same pipeline in a later step).
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

/// Authenticator service.
#[derive(Parser)]
#[command(name = "authenticator")]
#[command(about = "Insight authenticator — OIDC sessions + cookie-to-JWT exchange")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Path to the YAML configuration file.
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
    /// Validate configuration and exit.
    Check,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut config = AppConfig::load_or_default(cli.config.as_ref())?;
    config.apply_cli_overrides(cli.verbose);

    if cli.print_config {
        println!("Effective configuration:\n{}", config.to_yaml()?);
        return Ok(());
    }

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run_server(config).await,
        Commands::Check => {
            // Loading + parsing the config already validated its shape.
            println!("configuration OK");
            Ok(())
        }
    }
}
