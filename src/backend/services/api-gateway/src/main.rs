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

// Insight modules (compiled into the binary, registered via inventory).
mod auth_info;
mod core_types;
mod proxy;

// Link external modules via inventory — runtime discovers them automatically.
use api_gateway_module as _;
use authn_resolver as _;
use authz_resolver as _;
use grpc_hub as _;
use module_orchestrator as _;
use oidc_authn_plugin as _;
use single_tenant_tr_plugin as _;
use static_authz_plugin as _;
use tenant_resolver as _;
use types_registry as _;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use modkit::bootstrap::{AppConfig, run_migrate, run_server};

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

#[derive(Subcommand, Debug, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    type R = Result<(), Box<dyn std::error::Error>>;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    #[test]
    fn bare_invocation_has_no_subcommand_and_defaults_to_run() -> R {
        let cli = parse(&["insight-api-gateway"])?;
        assert!(cli.command.is_none());
        // main maps None → Run
        assert_eq!(cli.command.unwrap_or(Commands::Run), Commands::Run);
        assert_eq!(cli.verbose, 0);
        assert!(!cli.print_config);
        assert!(cli.config.is_none());
        Ok(())
    }

    #[test]
    fn subcommands_parse() -> R {
        assert_eq!(parse(&["x", "run"])?.command, Some(Commands::Run));
        assert_eq!(parse(&["x", "check"])?.command, Some(Commands::Check));
        assert_eq!(parse(&["x", "migrate"])?.command, Some(Commands::Migrate));
        Ok(())
    }

    #[test]
    fn verbose_flag_counts() -> R {
        assert_eq!(parse(&["x"])?.verbose, 0);
        assert_eq!(parse(&["x", "-v"])?.verbose, 1);
        assert_eq!(parse(&["x", "-vv"])?.verbose, 2);
        Ok(())
    }

    #[test]
    fn config_path_and_print_config_flags() -> R {
        let cli = parse(&["x", "--config", "/etc/insight.yaml", "--print-config"])?;
        assert_eq!(
            cli.config.as_deref(),
            Some(std::path::Path::new("/etc/insight.yaml"))
        );
        assert!(cli.print_config);
        Ok(())
    }

    #[test]
    fn short_config_flag_works() -> R {
        let cli = parse(&["x", "-c", "a.yaml", "run"])?;
        assert_eq!(cli.config.as_deref(), Some(std::path::Path::new("a.yaml")));
        assert_eq!(cli.command, Some(Commands::Run));
        Ok(())
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["x", "bogus"]).is_err());
    }

    #[test]
    fn cli_definition_is_valid() {
        // Catches clap derive misconfig (duplicate args, bad defaults) at test time.
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
