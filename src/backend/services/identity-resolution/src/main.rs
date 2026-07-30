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
mod migration;
mod seed_runner;

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
use clap::{Parser, Subcommand};
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

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the server (default).
    Run,
    /// Apply pending schema migrations + the first-admin bootstrap and exit.
    /// The Helm chart runs this as an initContainer before the server pod
    /// (same pattern as the analytics service).
    Migrate,
    /// Run one persons-seed and exit (issue #1690). The Helm chart runs this
    /// as a `CronJob`; operators run it manually via `kubectl create job
    /// --from=cronjob/...`. Exit codes: 0 ok / 1 failed / 2 another run holds
    /// the lock / 3 refused by an input guard.
    Seed {
        /// Seed mode; only `link-by-email` is implemented.
        #[arg(long, default_value = seed_runner::LINK_BY_EMAIL_MODE)]
        mode: String,
        /// Override the input guards (empty `identity_inputs` / wrong-tenant).
        #[arg(long)]
        force: bool,
    },
}

/// Exit codes of the `seed` subcommand, mirrored in the Job monitoring docs.
const EXIT_SEED_FAILED: i32 = 1;
const EXIT_SEED_LOCK_BUSY: i32 = 2;
const EXIT_SEED_GUARD: i32 = 3;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // Layered config: defaults -> YAML -> env (APP__*). Logging/OTel are
    // initialized by the bootstrap runtime for the server path; subcommands
    // run outside it and install their own plain subscriber.
    let config = AppConfig::load_or_default(cli.config.as_ref())?;
    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run_server(config).await,
        Commands::Migrate => {
            init_subcommand_logging();
            gear::run_migrate(&config).await
        }
        Commands::Seed { mode, force } => {
            init_subcommand_logging();
            match gear::run_seed(&config, &mode, force).await {
                Ok(()) => Ok(()),
                Err(seed_runner::SeedRunError::LockBusy) => {
                    tracing::warn!("another persons-seed run holds the lock; exiting");
                    std::process::exit(EXIT_SEED_LOCK_BUSY);
                }
                Err(seed_runner::SeedRunError::Guard(msg)) => {
                    tracing::error!(%msg, "persons-seed refused by input guard");
                    std::process::exit(EXIT_SEED_GUARD);
                }
                Err(seed_runner::SeedRunError::Failed(e)) => {
                    tracing::error!(error = %format!("{e:#}"), "persons-seed failed");
                    std::process::exit(EXIT_SEED_FAILED);
                }
            }
        }
    }
}

/// Plain stdout logging for the `migrate` / `seed` subcommands. The bootstrap
/// runtime only installs its subscriber inside `run_server`, so without this
/// every `tracing::…` on the subcommand paths is a silent no-op — and the
/// seed Job's logs are half its observability. `try_init` keeps this safe if
/// a future toolkit starts initializing earlier.
fn init_subcommand_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
}
