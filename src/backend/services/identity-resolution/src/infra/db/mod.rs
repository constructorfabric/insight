//! `MariaDB` connection.
//!
//! **Self-managed `SeaORM` pool — we deliberately do NOT use the toolkit `db`
//! capability** (same as the analytics gear). The identity queries need SQL that
//! `cf-gears-toolkit-db` (v0.8.4) can neither express via its scoped
//! entity-builder nor run as raw SQL (it intentionally exposes no raw-SQL path —
//! `DbConn`/`DbTx` are builder-only). Specifically:
//!   * window functions (`ROW_NUMBER()` / `LEAD() OVER (…)`) — the resolver reads
//!     and the SCD2 `account_person_map` / `org_chart` rebuilds;
//!   * `WITH RECURSIVE` — the org-subchart / visibility traversals;
//!   * atomic conditional DML with a correlated subquery — the role in-use and
//!     last-admin lockout guards.
//!
//! See constructorfabric/gears-rust#4239 for the capability request.
//!
//! All SQL here is **verbatim from the .NET service** (cutover parity). It is
//! injection-safe despite being raw: every value is a **bound parameter**
//! (`Statement::from_sql_and_values`, no string interpolation) and the tenant is
//! always pinned in the `WHERE`. The `identity` database is owned by .NET today.

pub mod entities;
pub mod ops_repo;
pub mod persons_repo;
pub mod roles_repo;
pub mod seed_repo;

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
