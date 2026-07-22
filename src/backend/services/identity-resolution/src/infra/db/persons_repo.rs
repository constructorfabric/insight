//! Read queries against the identity store (`persons`).
//!
//! Ported from the .NET service's `Sql.Profiles.cs`. The resolution queries use
//! window functions (`ROW_NUMBER()` over the canonical partition) that have no
//! first-class SeaORM query-builder form and no `toolkit-db` equivalent (see
//! `infra::db` module docs + constructorfabric/gears-rust#4239), so we run them
//! as **raw SQL** via SeaORM's `Statement` and read columns off the
//! `QueryResult`. Running the same SQL as the .NET service keeps resolution
//! behaviour identical.

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
        [
            tenant_id.as_bytes().to_vec().into(),
            email.trim().to_owned().into(),
        ],
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

/// One current source-native id for a person (repo-level row). The domain maps
/// it to the API `ProfileIdEntry` — the DB layer stays free of API types, the
/// same way `assemble_profile` maps `persons::Model` to the response.
pub struct SourceIdRow {
    pub source_type: String,
    pub source_id: Uuid,
    pub value: String,
}

/// All current source-native ids for one person — one row per source instance
/// (latest `value_type='id'` per (tenant, person, `source_type`, `source_id`)),
/// ordered by source. Ported from `Sql.Profiles.cs::CurrentSourceIdsForPerson`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored `insight_source_id` is not
/// 16 bytes.
pub async fn current_source_ids_for_person(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<Vec<SourceIdRow>> {
    // Verbatim from Sql.Profiles.cs::CurrentSourceIdsForPerson, `@param` -> `?`.
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                insight_source_type,
                insight_source_id,
                value_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id = ?
              AND person_id         = ?
              AND value_type        = 'id'
        )
        SELECT insight_source_type, insight_source_id, value_id AS value
        FROM ranked
        WHERE rn = 1
        ORDER BY insight_source_type, insight_source_id
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            person_id.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all(stmt).await?;
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let source_id_bytes: Vec<u8> = row.try_get("", "insight_source_id")?;
        // `value_type='id'` rows always carry `value_id` in practice; treat a
        // NULL defensively as empty rather than dropping the source instance.
        let value: Option<String> = row.try_get("", "value")?;
        ids.push(SourceIdRow {
            source_type,
            source_id: Uuid::from_slice(&source_id_bytes)?,
            value: value.unwrap_or_default(),
        });
    }
    Ok(ids)
}

/// One current parent edge for a child, scoped to one source instance
/// (repo-level row). Ported from the .NET `OrgChartEdge`.
pub struct OrgChartEdge {
    pub source_type: String,
    pub source_id: Uuid,
    pub parent_person_id: Uuid,
}

/// Current parent edges for one child (`valid_to IS NULL`), across every source
/// instance, ordered by source. The caller filters to the configured
/// `org_chart` source. Ported from `Sql.OrgChart.cs::CurrentParentsForChild`.
///
/// The `parent_person_id IS NOT NULL` filter intentionally diverges from .NET:
/// the seed writes Path-B root/membership rows with a NULL parent, and decoding
/// those into a non-nullable id would 500 the profile — a parent edge with no
/// parent is not an edge, so it is skipped.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id column is not 16 bytes.
pub async fn current_parents_for_child(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    child_person_id: Uuid,
) -> anyhow::Result<Vec<OrgChartEdge>> {
    const SQL: &str = r"
        SELECT insight_source_type, insight_source_id, parent_person_id
        FROM org_chart
        WHERE insight_tenant_id = ?
          AND child_person_id   = ?
          AND valid_to IS NULL
          AND parent_person_id IS NOT NULL
        ORDER BY insight_source_type, insight_source_id
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            child_person_id.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all(stmt).await?;
    let mut edges = Vec::with_capacity(rows.len());
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let source_id: Vec<u8> = row.try_get("", "insight_source_id")?;
        let parent_person_id: Vec<u8> = row.try_get("", "parent_person_id")?;
        edges.push(OrgChartEdge {
            source_type,
            source_id: Uuid::from_slice(&source_id)?,
            parent_person_id: Uuid::from_slice(&parent_person_id)?,
        });
    }
    Ok(edges)
}

/// One current child edge for a parent (repo-level row). Only the fields the
/// subordinates expansion needs: the source it came from and the child id.
pub struct OrgChartChildEdge {
    pub source_type: String,
    pub child_person_id: Uuid,
}

/// Current direct-children edges for one parent (`valid_to IS NULL`), across
/// every source instance, ordered by source then child. The caller filters to
/// the configured source and de-dupes. Ported from
/// `Sql.OrgChart.cs::CurrentChildrenForParent`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id column is not 16 bytes.
pub async fn current_children_for_parent(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    parent_person_id: Uuid,
) -> anyhow::Result<Vec<OrgChartChildEdge>> {
    const SQL: &str = r"
        SELECT insight_source_type, child_person_id
        FROM org_chart
        WHERE insight_tenant_id  = ?
          AND parent_person_id   = ?
          AND valid_to IS NULL
        ORDER BY insight_source_type, insight_source_id, child_person_id
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            parent_person_id.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all(stmt).await?;
    let mut edges = Vec::with_capacity(rows.len());
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let child_person_id: Vec<u8> = row.try_get("", "child_person_id")?;
        edges.push(OrgChartChildEdge {
            source_type,
            child_person_id: Uuid::from_slice(&child_person_id)?,
        });
    }
    Ok(edges)
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
    /// Set `IDENTITY_TEST_DB_URL` + `IDENTITY_TEST_TENANT_ID` + `IDENTITY_TEST_EMAIL`
    /// (a known email in that tenant) and a MariaDB port-forward to run; skips
    /// cleanly otherwise so CI stays green. The email is not hardcoded so the
    /// test carries no real address and isn't tied to one person.
    #[tokio::test]
    async fn resolve_by_email_against_dev_db() -> anyhow::Result<()> {
        let (Ok(url), Ok(tenant_raw), Ok(known_email)) = (
            std::env::var("IDENTITY_TEST_DB_URL"),
            std::env::var("IDENTITY_TEST_TENANT_ID"),
            std::env::var("IDENTITY_TEST_EMAIL"),
        ) else {
            eprintln!(
                "skip: set IDENTITY_TEST_DB_URL + IDENTITY_TEST_TENANT_ID + IDENTITY_TEST_EMAIL to run"
            );
            return Ok(());
        };
        let tenant = Uuid::parse_str(tenant_raw.trim())?;
        let conn = db::connect(&url).await?;

        let known = resolve_person_ids_by_email(&conn, tenant, known_email.trim()).await?;
        assert_eq!(
            known.len(),
            1,
            "known email should resolve to exactly one person"
        );

        let missing = resolve_person_ids_by_email(&conn, tenant, "nobody@nowhere.invalid").await?;
        assert!(
            missing.is_empty(),
            "unknown email should resolve to zero persons"
        );
        Ok(())
    }
}
