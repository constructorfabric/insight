//! First-admin bootstrap — port of the .NET `BootstrapAdminRunner`.
//!
//! Breaks the chicken-and-egg between the admin-gated CRUD endpoints and an
//! empty `person_roles` table: when `bootstrap_admin_person_id` is configured,
//! the person gets an active `admin` assignment in `tenant_default_id`.
//! Idempotent (`INSERT … WHERE NOT EXISTS` on the active-assignment triple);
//! runs at the end of the `migrate` subcommand — the .NET service ran it at
//! startup after `DbUp`, same effective point in the lifecycle.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use uuid::Uuid;

use super::roles_repo::ADMIN_ROLE_ID;
use crate::config::GearConfig;

/// SQL is verbatim from `BootstrapAdminRunner.cs` (named params → `?`).
const SQL: &str = "
    INSERT INTO person_roles
        (person_role_id, insight_tenant_id, person_id, role_id,
         valid_from, valid_to, author_person_id, reason)
    SELECT
        ?, ?, ?, ?,
        UTC_TIMESTAMP(6), NULL, ?, 'bootstrap'
    WHERE NOT EXISTS (
        SELECT 1 FROM person_roles
        WHERE insight_tenant_id = ?
          AND person_id         = ?
          AND role_id           = ?
          AND valid_to IS NULL
    )
";

/// Seed the first admin according to the gear config. Mirrors the .NET
/// skip semantics: no bootstrap person configured → silent no-op; person
/// configured but no tenant → warn and skip.
///
/// # Errors
///
/// Returns an error on an unparseable configured UUID or a database failure.
pub async fn bootstrap_admin(db: &DatabaseConnection, config: &GearConfig) -> anyhow::Result<()> {
    if config.bootstrap_admin_person_id.is_empty() {
        return Ok(());
    }
    if config.tenant_default_id.is_empty() {
        tracing::warn!("bootstrap admin requested but tenant_default_id is not set — skipping");
        return Ok(());
    }
    let person = Uuid::parse_str(config.bootstrap_admin_person_id.trim())
        .map_err(|e| anyhow::anyhow!("invalid bootstrap_admin_person_id: {e}"))?;
    let tenant = Uuid::parse_str(config.tenant_default_id.trim())
        .map_err(|e| anyhow::anyhow!("invalid tenant_default_id: {e}"))?;

    let person_role_id = Uuid::now_v7();
    let result = db
        .execute(Statement::from_sql_and_values(
            DbBackend::MySql,
            SQL,
            [
                person_role_id.as_bytes().to_vec().into(),
                tenant.as_bytes().to_vec().into(),
                person.as_bytes().to_vec().into(),
                ADMIN_ROLE_ID.as_bytes().to_vec().into(),
                person.as_bytes().to_vec().into(),
                tenant.as_bytes().to_vec().into(),
                person.as_bytes().to_vec().into(),
                ADMIN_ROLE_ID.as_bytes().to_vec().into(),
            ],
        ))
        .await?;

    if result.rows_affected() > 0 {
        tracing::info!(%tenant, %person, "bootstrap admin role inserted");
    } else {
        tracing::info!(%tenant, %person, "bootstrap admin role already present — skipped");
    }
    Ok(())
}
