mod api;
mod config;
mod gear;
mod infra;
mod migration;

use std::path::PathBuf;

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

use anyhow::Result;
use clap::{Parser, Subcommand};
use toolkit::bootstrap::{AppConfig, run_server};

#[derive(Debug, Parser)]
#[command(name = "connectors", about = "Connector management service", version)]
struct Cli {
    #[arg(short, long)]
    config: Option<PathBuf>,
    #[arg(long)]
    print_config: bool,
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Run,
    Migrate,
    Check,
    Openapi,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Commands::Run);
    if matches!(command, Commands::Openapi) {
        let document = api::openapi_document()?;
        print!("{}", insight_openapi::canonical_json(&document)?);
        return Ok(());
    }

    let mut config = AppConfig::load_or_default(cli.config.as_ref())?;
    config.apply_cli_overrides(cli.verbose);
    if cli.print_config {
        println!("{}", config::redacted_yaml(&config)?);
        return Ok(());
    }

    match command {
        Commands::Run => {
            let resource = &config.opentelemetry.resource;
            insight_log_context::init_identity_from_resource(
                &resource.service_name,
                &resource.attributes,
            );
            run_server(config).await
        }
        Commands::Migrate => {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .try_init();
            gear::run_migrate(&config).await
        }
        Commands::Check => gear::check_config(&config),
        Commands::Openapi => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn committed_openapi_document_is_current() -> anyhow::Result<()> {
        let document = super::api::openapi_document()?;
        insight_openapi::check_committed(
            &document,
            env!("CARGO_MANIFEST_DIR"),
            env!("CARGO_PKG_NAME"),
        )?;
        Ok(())
    }
}
