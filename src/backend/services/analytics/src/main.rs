mod api;
mod config;
mod domain;
mod gear;
mod infra;
mod mcp;
mod migration;
mod sql_explorer;

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

/// Analytics API service.
#[derive(Parser)]
#[command(name = "analytics")]
#[command(about = "Insight Analytics API — query service over `ClickHouse` metrics")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Path to YAML configuration file.
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
    /// Run database migrations + startup probes and exit.
    Migrate,
    /// Validate configuration and exit.
    Check,
    /// Print the OpenAPI document to stdout and exit. Built offline from the
    /// route table — no database, no HTTP listener, no config needed. Used to
    /// regenerate docs/components/backend/analytics/openapi.json and to
    /// drift-check it in CI.
    Openapi,
    /// Print the metric passports document to stdout and exit. Built offline
    /// from the embedded registry — no database, no config. Used to regenerate
    /// the committed passports.md next to the registry; a drift test guards it.
    Passports,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut config = AppConfig::load_or_default(cli.config.as_ref())?;
    config.apply_cli_overrides(cli.verbose);

    if cli.print_config {
        println!(
            "Effective configuration:\n{}",
            config::redacted_yaml(&config)?
        );
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
        Commands::Migrate => {
            init_subcommand_logging();
            gear::run_migrate(&config).await
        }
        Commands::Check => gear::check_config(&config),
        Commands::Openapi => print_openapi(),
        Commands::Passports => {
            print_passports();
            Ok(())
        }
    }
}

// WORKAROUND: the bootstrap runtime installs its subscriber only inside
// `run_server`, so every `tracing::…` on the subcommand path is otherwise a no-op.
fn init_subcommand_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
}

fn print_openapi() -> Result<()> {
    let doc = api::openapi_document()?;
    print!("{}", insight_openapi::canonical_json(&doc)?);
    Ok(())
}

fn print_passports() {
    print!(
        "{}",
        domain::metric_definitions::passport::render_passports()
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn print_openapi_writes_the_document() -> anyhow::Result<()> {
        super::print_openapi()
    }

    #[test]
    fn committed_openapi_document_is_current() -> anyhow::Result<()> {
        let doc = super::api::openapi_document()?;
        insight_openapi::check_committed(&doc, env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"))?;
        Ok(())
    }

    #[test]
    fn print_passports_writes_the_document() {
        super::print_passports();
    }
}
