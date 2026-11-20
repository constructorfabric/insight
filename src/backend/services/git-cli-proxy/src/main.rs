// `use … as _;` is load-bearing: the gears register through `inventory` at
// link time, so an unreferenced crate is dropped and never registers.
use git_cli_proxy as _;
use grpc_hub as _;
use service_discovery as _;
use types_registry as _;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use toolkit::bootstrap::{AppConfig, run_server};

/// Git CLI Proxy service.
#[derive(Parser)]
#[command(name = "git-cli-proxy")]
#[command(about = "Insight Git CLI Proxy — clone-based git data extraction")]
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
    /// Print the `OpenAPI` document as JSON and exit.
    Openapi,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();
    let command = cli.command.take().unwrap_or(Commands::Run);

    let load_config = || AppConfig::load_or_default(cli.config.as_ref());

    match command {
        Commands::Run => {
            let config = load_config()?;
            let resource = &config.opentelemetry.resource;
            insight_log_context::init_identity_from_resource(
                &resource.service_name,
                &resource.attributes,
            );
            run_server(config).await
        }
        Commands::Openapi => print_openapi(),
    }
}

fn print_openapi() -> Result<()> {
    let doc = git_cli_proxy::api::openapi_document()?;
    print!("{}", insight_openapi::canonical_json(&doc)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn committed_openapi_document_is_current() -> anyhow::Result<()> {
        let doc = git_cli_proxy::api::openapi_document()?;
        insight_openapi::check_committed(&doc, env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"))?;
        Ok(())
    }
}
