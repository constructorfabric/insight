//! Analytics API — read-only query service over predefined `ClickHouse` metrics.
//!
//! Serves admin-defined metrics (SQL queries stored in `MariaDB`) with tenant-scoped,
//! org-scoped security filters and `OData`-style querying.
//!
//! # Usage
//!
//! ```text
//! analytics-api --config config.yaml
//! analytics-api --config config.yaml migrate
//! ```

mod api;
mod auth;
mod config;
mod domain;
mod infra;
mod migration;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

/// Analytics API service.
#[derive(Parser)]
#[command(name = "analytics-api")]
#[command(about = "Insight Analytics API — query service over `ClickHouse` metrics")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Path to YAML configuration file.
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
    // Initialize tracing
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
    tracing::info!("starting analytics-api");

    // Connect to MariaDB
    let db = infra::db::connect(&cfg.database_url).await?;

    // Run pending migrations
    infra::db::run_migrations(&db).await?;

    // Connect to ClickHouse
    let mut ch_config =
        insight_clickhouse::Config::new(&cfg.clickhouse_url, &cfg.clickhouse_database);
    if let (Some(user), Some(password)) = (&cfg.clickhouse_user, &cfg.clickhouse_password) {
        ch_config = ch_config.with_auth(user, password);
    }
    let ch = insight_clickhouse::Client::new(ch_config);

    // Identity Resolution client
    let identity =
        infra::identity_resolution::IdentityResolutionClient::new(&cfg.identity_resolution_url);

    // Build app state
    let state = api::AppState {
        db,
        ch,
        identity,
        config: cfg.clone(),
    };

    // Build router
    let app = api::router(state);

    // Start server
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
