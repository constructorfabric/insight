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

    // Refuse to start if any required CHECK constraint is missing. Our
    // bitnami-shipped MariaDB is 11.x, but on customer-managed DBs (BYO-DB
    // installs, RDS, Cloud SQL, on-prem) we can't audit the version or
    // `sql_mode`. See `infra/db/check_probe` and DESIGN §2.2
    // `cpt-metric-cat-constraint-mariadb-check` for the full rationale.
    infra::db::check_probe::assert_required_checks(&db).await?;

    // Connect to ClickHouse
    let mut ch_config =
        insight_clickhouse::Config::new(&cfg.clickhouse_url, &cfg.clickhouse_database);
    if let (Some(user), Some(password)) = (&cfg.clickhouse_user, &cfg.clickhouse_password) {
        ch_config = ch_config.with_auth(user, password);
    }
    let ch = insight_clickhouse::Client::new(ch_config);

    // Identity client
    let identity = infra::identity::IdentityClient::new(&cfg.identity_url);

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

    // Same probe as `run_server`. An operator running `analytics-api migrate`
    // after a schema rollback wants the integrity signal too — the typical
    // recovery path is `migrate` first, restart the service second, and
    // dropping the probe here would silently re-greenlight a DB that's
    // missing a CHECK the application relies on.
    infra::db::check_probe::assert_required_checks(&db).await?;

    tracing::info!("migrations complete");
    Ok(())
}
