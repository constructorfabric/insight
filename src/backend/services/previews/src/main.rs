mod api;
mod config;
mod domain;
mod gear;
mod infra;
mod sweep;

// `use … as _;` is load-bearing: the gears register through `inventory` at
// link time, so an unreferenced crate is dropped and never registers.
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
        Commands::Run => {
            let config = AppConfig::load_or_default(cli.config.as_ref())?;
            let resource = &config.opentelemetry.resource;
            insight_log_context::init_identity_from_resource(
                &resource.service_name,
                &resource.attributes,
            );
            run_server(config).await
        }
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
