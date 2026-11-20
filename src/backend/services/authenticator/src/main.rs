// Pedantic style lint: backticking every spec and header name in prose hurts readability.
#![allow(clippy::doc_markdown)]

mod api;
mod audit;
mod backchannel;
mod config;
mod cookie;
mod csrf;
mod gear;
mod identity;
mod issuers;
mod janitor;
mod jwt;
mod local_client;
mod mcp_oauth;
mod oidc;
mod ratelimit;
mod refresher;
mod service_token;
mod session;

// System gears — linked via inventory for the REST host + auth pipeline.
use api_gateway as _;
use authn_resolver as _;
use authz_resolver as _;
use grpc_hub as _;
use oidc_authn_plugin as _;
use service_discovery as _;
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
    /// Print the OpenAPI document to stdout and exit. Built offline from the
    /// route table — no Redis, IdP, or config needed. Used to regenerate
    /// docs/components/backend/authenticator/openapi.json and to drift-check
    /// it in CI.
    Openapi,
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
        Commands::Run => {
            let resource = &config.opentelemetry.resource;
            insight_log_context::init_identity_from_resource(
                &resource.service_name,
                &resource.attributes,
            );
            run_server(config).await
        }
        Commands::Check => {
            println!("configuration OK");
            Ok(())
        }
        Commands::Openapi => print_openapi(),
    }
}

fn print_openapi() -> Result<()> {
    let doc = api::openapi_document()?;
    print!("{}", insight_openapi::canonical_json(&doc)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn committed_openapi_document_is_current() -> anyhow::Result<()> {
        let doc = super::api::openapi_document()?;
        insight_openapi::check_committed(&doc, env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"))?;
        Ok(())
    }
}
