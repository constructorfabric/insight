//! Read queries against the identity store (`persons`).
//!
//! Ported from the .NET service's `Sql.Profiles.cs`. The resolution queries use
//! window functions (`ROW_NUMBER()` over the canonical partition) that have no
//! first-class SeaORM query-builder form, so we run them as **raw SQL** via
//! SeaORM's `Statement` and read columns off the `QueryResult`. Running the same
//! SQL as the .NET service keeps resolution behaviour identical.

use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, QueryFilter,
    QueryResult, Statement,
};
use uuid::Uuid;

use super::entities::persons;

/// Resolve the set of `person_id`s whose CURRENT email (latest observation per
/// source instance) equals `email` within the tenant.
///
/// The caller maps the result to the contract: 0 rows → 404 `person_not_found`,
/// 1 → resolved, >1 → 422 `ambiguous_profile`.
///
/// Case handling matches the .NET service (ADR-0011): the input is trimmed
/// only — the `value_id` column collation does case-insensitive matching.
///
/// # Errors
///
/// Returns an error if the query fails or a stored `person_id` is not 16 bytes.
pub async fn resolve_person_ids_by_email(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    email: &str,
) -> anyhow::Result<Vec<Uuid>> {
    // Verbatim from Sql.Profiles.cs::ResolvePersonIdsByEmail, `@param` -> `?`.
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                person_id,
                value_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id = ?
              AND value_type = 'email'
        )
        SELECT DISTINCT person_id
        FROM ranked
        WHERE rn = 1
          AND value_id = ?
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [tenant_id.as_bytes().to_vec().into(), email.trim().to_owned().into()],
    );

    let rows = db.query_all(stmt).await?;
    person_ids_from_rows(rows)
}

/// Resolve the set of `person_id`s whose CURRENT `value_type='id'` observation
/// on the given source instance (`source_type` + `source_id`) equals `value`.
/// Source-instance scoped, ported from .NET `Sql.Profiles.cs::ResolvePersonIdsBySourceId`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored `person_id` is not 16 bytes.
pub async fn resolve_person_ids_by_source_id(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    source_type: &str,
    source_id: Uuid,
    value: &str,
) -> anyhow::Result<Vec<Uuid>> {
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                person_id,
                value_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id   = ?
              AND insight_source_type = ?
              AND insight_source_id   = ?
              AND value_type          = 'id'
        )
        SELECT DISTINCT person_id
        FROM ranked
        WHERE rn = 1
          AND value_id = ?
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            source_type.to_owned().into(),
            source_id.as_bytes().to_vec().into(),
            // Source-native ids are matched as-is (the .NET service trims only
            // email, not the id path).
            value.to_owned().into(),
        ],
    );

    let rows = db.query_all(stmt).await?;
    person_ids_from_rows(rows)
}

/// Fetch every observation row for a person within the tenant (all value types,
/// all sources). The caller collapses them to the current value per attribute.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn fetch_person_observations(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<Vec<persons::Model>> {
    let rows = persons::Entity::find()
        .filter(persons::Column::InsightTenantId.eq(tenant_id.as_bytes().to_vec()))
        .filter(persons::Column::PersonId.eq(person_id.as_bytes().to_vec()))
        .all(db)
        .await?;
    Ok(rows)
}

/// Read the `person_id` (`binary(16)`) column off each result row into a `Uuid`.
fn person_ids_from_rows(rows: Vec<QueryResult>) -> anyhow::Result<Vec<Uuid>> {
    let mut person_ids = Vec::with_capacity(rows.len());
    for row in rows {
        let bytes: Vec<u8> = row.try_get("", "person_id")?;
        person_ids.push(Uuid::from_slice(&bytes)?);
    }
    Ok(person_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db;

    /// Integration test against a live MariaDB. Data-dependent (dev cluster).
    /// Set `IDENTITY_TEST_DB_URL` + `IDENTITY_TEST_TENANT_ID` (and a MariaDB
    /// port-forward) to run; skips cleanly otherwise so CI stays green.
    #[tokio::test]
    async fn resolve_by_email_against_dev_db() -> anyhow::Result<()> {
        let (Ok(url), Ok(tenant_raw)) = (
            std::env::var("IDENTITY_TEST_DB_URL"),
            std::env::var("IDENTITY_TEST_TENANT_ID"),
        ) else {
            eprintln!("skip: set IDENTITY_TEST_DB_URL + IDENTITY_TEST_TENANT_ID to run");
            return Ok(());
        };
        let tenant = Uuid::parse_str(tenant_raw.trim())?;
        let conn = db::connect(&url).await?;

        let known =
            resolve_person_ids_by_email(&conn, tenant, "serdar.findik@constructor.tech").await?;
        assert_eq!(known.len(), 1, "known email should resolve to exactly one person");

        let missing =
            resolve_person_ids_by_email(&conn, tenant, "nobody@nowhere.invalid").await?;
        assert!(missing.is_empty(), "unknown email should resolve to zero persons");
        Ok(())
    }
}
