//! Identity Resolution -- entry point.
//!
//! Owns its `MariaDB` schema (the `identity` database) via an embedded
//! `SeaORM` `Migrator`. On `run`, migrations are applied before serving;
//! the `migrate` subcommand runs them and exits (used as a helm init
//! container). See ADR-0006 for the service-owned-migrations decision.

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use identity_resolution::{PeopleStore, build_router, config, infra};

#[derive(Parser)]
#[command(name = "identity-resolution")]
#[command(about = "Identity Resolution -- person lookup + MariaDB-backed identity history")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[arg(short, long)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the server (default).
    Run,
    /// Run database migrations and exit.
    Migrate,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let cli = Cli::parse();
    let cfg = config::AppConfig::load(cli.config.as_deref())?;

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run_server(cfg).await,
        Commands::Migrate => run_migrate(cfg).await,
    }
}

async fn run_server(cfg: config::AppConfig) -> anyhow::Result<()> {
    tracing::info!("starting identity-resolution");

    // Apply any pending MariaDB migrations before serving. Idempotent --
    // if the init container already ran `migrate`, this is a no-op.
    let db = infra::db::connect(&cfg.database_url).await?;
    infra::db::run_migrations(&db).await?;

    let mut ch_config =
        insight_clickhouse::Config::new(&cfg.clickhouse_url, &cfg.clickhouse_database);
    if let (Some(user), Some(password)) = (&cfg.clickhouse_user, &cfg.clickhouse_password) {
        ch_config = ch_config.with_auth(user, password);
    }
    let ch = insight_clickhouse::Client::new(ch_config);

    let store = PeopleStore::load(&ch).await?;
    tracing::info!(count = store.len(), "people loaded into memory");

    let app = build_router(store);

    let addr = cfg.bind_addr.parse::<std::net::SocketAddr>()?;
    tracing::info!(addr = %addr, "listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn run_migrate(cfg: config::AppConfig) -> anyhow::Result<()> {
    tracing::info!("running migrations");
    let db = infra::db::connect(&cfg.database_url).await?;
    infra::db::run_migrations(&db).await?;
    tracing::info!("migrations complete");
    Ok(())
}
