//! `MariaDB` connection.
//!
//! Self-managed `SeaORM` pool (same approach as the analytics gear — we do NOT
//! use the toolkit `db` capability). The `identity` database is owned by the
//! .NET service today; we read `persons` / `account_person_map` from it.

pub mod entities;
pub mod persons_repo;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};

/// Connect to `MariaDB` and return a connection pool.
///
/// # Errors
///
/// Returns an error if the connection cannot be established.
pub async fn connect(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut opts = ConnectOptions::new(database_url);
    opts.max_connections(10)
        .min_connections(2)
        .sqlx_logging(false);

    let db = Database::connect(opts).await?;
    tracing::info!("connected to MariaDB");
    Ok(db)
}
