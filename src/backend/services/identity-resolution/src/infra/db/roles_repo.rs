//! Role checks against the shared MariaDB `person_roles` table.
//!
//! Ported from the .NET `RolesRepository` / `Sql.Roles.cs`. Used by the
//! persons-seed admin gate — only admins may trigger a seed, matching the .NET
//! `CallerAdminCheck`.

#![allow(dead_code)]

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use uuid::Uuid;

/// The `admin` role id — a stable seed value (`Roles.Admin` in the .NET domain).
pub const ADMIN_ROLE_ID: Uuid = Uuid::from_u128(0xa4d1_1000_0000_4000_8000_0000_0000_0001);

/// True if `person_id` currently holds an active (`valid_to IS NULL`) `role_id`
/// in the tenant. Mirrors `Sql.Roles.cs::HasActivePersonRole` (`SELECT EXISTS`);
/// implemented as a `LIMIT 1` probe so the truthiness maps cleanly through
/// SeaORM regardless of how the driver types an `EXISTS` scalar.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn has_active_role(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
    role_id: Uuid,
) -> anyhow::Result<bool> {
    const SQL: &str = r"
        SELECT 1
        FROM person_roles
        WHERE insight_tenant_id = ?
          AND person_id         = ?
          AND role_id           = ?
          AND valid_to IS NULL
        LIMIT 1
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            person_id.as_bytes().to_vec().into(),
            role_id.as_bytes().to_vec().into(),
        ],
    );

    Ok(db.query_one(stmt).await?.is_some())
}

/// Convenience: does `person_id` hold the active `admin` role in the tenant?
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn has_active_admin(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<bool> {
    has_active_role(db, tenant_id, person_id, ADMIN_ROLE_ID).await
}
