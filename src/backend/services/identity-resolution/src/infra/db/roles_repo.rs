//! Role checks against the shared MariaDB `person_roles` table.
//!
//! Ported from the .NET `RolesRepository` / `Sql.Roles.cs`. Used by the
//! persons-seed admin gate — only admins may trigger a seed, matching the .NET
//! `CallerAdminCheck`.

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

/// One row of the global `roles` catalogue (no tenant, no audit columns).
#[derive(Debug, Clone)]
pub struct Role {
    pub role_id: Uuid,
    pub name: String,
}

fn row_to_role(r: &sea_orm::QueryResult) -> anyhow::Result<Role> {
    let role_id: Vec<u8> = r.try_get("", "role_id")?;
    Ok(Role {
        role_id: Uuid::from_slice(&role_id)?,
        name: r.try_get("", "name")?,
    })
}

/// Look up a role by (unique) name — the duplicate-name pre-check. Ported from
/// `SqlRoles.RoleByName`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id is not 16 bytes.
pub async fn get_by_name(db: &DatabaseConnection, name: &str) -> anyhow::Result<Option<Role>> {
    const SQL: &str = "SELECT role_id, name FROM roles WHERE name = ? LIMIT 1";
    let stmt = Statement::from_sql_and_values(DbBackend::MySql, SQL, [name.into()]);
    db.query_one(stmt)
        .await?
        .as_ref()
        .map(row_to_role)
        .transpose()
}

/// Look up a role by id. Ported from `SqlRoles.RoleById`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id is not 16 bytes.
pub async fn get_by_id(db: &DatabaseConnection, role_id: Uuid) -> anyhow::Result<Option<Role>> {
    const SQL: &str = "SELECT role_id, name FROM roles WHERE role_id = ? LIMIT 1";
    let stmt =
        Statement::from_sql_and_values(DbBackend::MySql, SQL, [role_id.as_bytes().to_vec().into()]);
    db.query_one(stmt)
        .await?
        .as_ref()
        .map(row_to_role)
        .transpose()
}

/// All roles, ordered by name. Ported from `SqlRoles.ListAllRoles`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id is not 16 bytes.
pub async fn list_all(db: &DatabaseConnection) -> anyhow::Result<Vec<Role>> {
    const SQL: &str = "SELECT role_id, name FROM roles ORDER BY name";
    let stmt = Statement::from_sql_and_values(DbBackend::MySql, SQL, []);
    db.query_all(stmt).await?.iter().map(row_to_role).collect()
}

/// Insert a new role. Ported from `SqlRoles.InsertRole`.
///
/// # Errors
///
/// Returns an error if the insert fails (e.g. duplicate `name`).
pub async fn insert_role(db: &DatabaseConnection, role_id: Uuid, name: &str) -> anyhow::Result<()> {
    const SQL: &str = "INSERT INTO roles (role_id, name) VALUES (?, ?)";
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [role_id.as_bytes().to_vec().into(), name.into()],
    );
    db.execute(stmt).await?;
    Ok(())
}

/// Hard-delete a role only if no ACTIVE `person_roles` reference it (single
/// atomic statement — the in-use guard). Returns rows affected: 1 = deleted,
/// 0 = missing or in use. Ported verbatim from `SqlRoles.TryDeleteRoleIfUnused`.
/// The correlated `NOT EXISTS` guard has no `toolkit-db` builder form → raw SQL
/// (see `infra::db` module docs + constructorfabric/gears-rust#4239).
///
/// # Errors
///
/// Returns an error if the delete fails.
pub async fn try_delete_if_unused(db: &DatabaseConnection, role_id: Uuid) -> anyhow::Result<u64> {
    const SQL: &str = r"
        DELETE FROM roles
        WHERE role_id = ?
          AND NOT EXISTS (
              SELECT 1 FROM person_roles WHERE role_id = ? AND valid_to IS NULL
          )
    ";
    let bytes = role_id.as_bytes().to_vec();
    let stmt =
        Statement::from_sql_and_values(DbBackend::MySql, SQL, [bytes.clone().into(), bytes.into()]);
    Ok(db.execute(stmt).await?.rows_affected())
}

/// Count active `person_roles` referencing a role across ALL tenants — feeds the
/// in-use error message. Ported from `SqlRoles.CountActivePersonRolesByRoleAnyTenant`.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn count_active_assignments_any_tenant(
    db: &DatabaseConnection,
    role_id: Uuid,
) -> anyhow::Result<i64> {
    const SQL: &str =
        "SELECT COUNT(*) AS c FROM person_roles WHERE role_id = ? AND valid_to IS NULL";
    let stmt =
        Statement::from_sql_and_values(DbBackend::MySql, SQL, [role_id.as_bytes().to_vec().into()]);
    match db.query_one(stmt).await? {
        Some(row) => Ok(row.try_get::<i64>("", "c")?),
        None => Ok(0),
    }
}
