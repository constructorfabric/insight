mod api;
mod chat;
mod config;
mod domain;
mod gear;
mod mcp;
mod migration;
mod store;

#[cfg(test)]
mod window_live_tests;

#[cfg(test)]
mod folders_live_tests;

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

#[derive(Debug, Parser)]
#[command(name = "insight-v3-core")]
#[command(about = "Insight v3 Core")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[arg(short, long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Run,
    Migrate,
    /// Print the `OpenAPI` document and exit. Offline — see
    /// [`api::openapi_document`].
    Openapi,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = AppConfig::load_or_default(cli.config.as_ref())?;

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => {
            let resource = &config.opentelemetry.resource;
            insight_log_context::init_identity_from_resource(
                &resource.service_name,
                &resource.attributes,
            );
            run_server(config).await
        }
        Commands::Migrate => {
            init_subcommand_logging();
            gear::run_migrate(&config).await
        }
        // No logging subscriber on this path: stdout stays pure JSON for the
        // drift gate to read.
        Commands::Openapi => {
            let document = api::openapi_document()?;
            print!("{}", insight_openapi::canonical_json(&document)?);
            Ok(())
        }
    }
}

/// Plain stdout logging for `migrate`, which runs outside the bootstrap server.
fn init_subcommand_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
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
