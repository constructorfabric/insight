//! Depth-bounded org subchart reads (recursive CTEs over `org_chart`).
//!
//! Ported from the .NET `SubchartRepository` / `Sql.Subchart.cs` (#348 / #344)
//! plus the visibility predicate `Sql.Visibility.cs::IsTargetInVisibleSet`. Every
//! query is a `WITH RECURSIVE` traversal, and the latest-observation pass uses
//! `ROW_NUMBER()`; neither construct has a `toolkit-db` builder or raw-SQL path,
//! so we run raw SQL on the self-managed pool (see `infra::db` module docs +
//! constructorfabric/gears-rust#4239). The .NET SQL uses named `@params` that
//! repeat on every recursion level, so we expand them to positional `?` via
//! [`super::sql_named::bind_named`]. Values are bound parameters (never
//! interpolated) and the tenant is always pinned in the `WHERE`.
//!
//! Result rows are flat (`person_id`, `parent_person_id`, the four attribute
//! fields); the tree is assembled in [`crate::domain::subchart`], mirroring the
//! .NET service split (`SqlSubchart` returns a flat set, `SubchartService`
//! builds the tree). Roots always surface with `parent_person_id IS NULL`.

use sea_orm::prelude::DateTime;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, Value};
use uuid::Uuid;

use super::sql_named::bind_named;

/// One flat node of a subchart traversal. `parent_person_id` is `None` for a
/// root (the anchor row / a forest top). The four attribute fields are the
/// latest observation of that `value_type`, or `None` when none exists.
#[derive(Debug, Clone)]
pub struct SubchartFlatNode {
    pub person_id: Uuid,
    pub parent_person_id: Option<Uuid>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub job_title: Option<String>,
    pub status: Option<String>,
}

fn row_to_flat(r: &sea_orm::QueryResult) -> anyhow::Result<SubchartFlatNode> {
    let person: Vec<u8> = r.try_get("", "person_id")?;
    let parent: Option<Vec<u8>> = r.try_get("", "parent_person_id")?;
    Ok(SubchartFlatNode {
        person_id: Uuid::from_slice(&person)?,
        parent_person_id: parent.map(|b| Uuid::from_slice(&b)).transpose()?,
        email: r.try_get("", "email")?,
        display_name: r.try_get("", "display_name")?,
        job_title: r.try_get("", "job_title")?,
        status: r.try_get("", "status")?,
    })
}

/// Predicate "can `viewer_person_id` see `target_person_id`?" as of `valid_at`
/// (`None` = right now). Ported from `IsTargetInVisibleSet`: a single recursive
/// CTE that unions the viewer, their active grant targets, the target itself
/// when a whole-tenant (wildcard) grant exists, and every `org_chart` descendant
/// of anyone already visible. The .NET final `SELECT EXISTS(…)` is rendered as a
/// `SELECT 1 … LIMIT 1` presence probe (truthiness = row present), matching the
/// `roles_repo::has_active_role` convention so it maps cleanly through SeaORM
/// regardless of how the driver types an `EXISTS` scalar.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn is_target_in_visible_set(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    viewer_person_id: Uuid,
    target_person_id: Uuid,
    org_source_type: &str,
    valid_at: Option<DateTime>,
) -> anyhow::Result<bool> {
    const SQL: &str = r"
        WITH RECURSIVE visible_set (person_id) AS (
            SELECT @viewer_person_id
            UNION
            SELECT viewed_person_id
            FROM visibility
            WHERE insight_tenant_id = @tenant_id
              AND viewer_person_id  = @viewer_person_id
              AND viewed_person_id  IS NOT NULL
              AND valid_from <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
              AND (valid_to IS NULL OR valid_to > COALESCE(@valid_at, UTC_TIMESTAMP(6)))
            UNION
            SELECT @target_person_id
            WHERE EXISTS (
                SELECT 1 FROM visibility
                WHERE insight_tenant_id = @tenant_id
                  AND viewer_person_id  = @viewer_person_id
                  AND viewed_person_id  IS NULL
                  AND valid_from <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
                  AND (valid_to IS NULL OR valid_to > COALESCE(@valid_at, UTC_TIMESTAMP(6)))
            )
            UNION
            SELECT oc.child_person_id
            FROM visible_set vs
            JOIN org_chart oc
              ON  oc.parent_person_id    = vs.person_id
              AND oc.insight_tenant_id   = @tenant_id
              AND oc.insight_source_type = @org_source_type
              AND oc.valid_from <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
              AND (oc.valid_to IS NULL OR oc.valid_to > COALESCE(@valid_at, UTC_TIMESTAMP(6)))
        )
        SELECT 1 FROM visible_set WHERE person_id = @target_person_id LIMIT 1
    ";

    let (sql, values) = bind_named(
        SQL,
        &[
            ("viewer_person_id", bytes(viewer_person_id)),
            ("tenant_id", bytes(tenant_id)),
            ("valid_at", valid_at.into()),
            ("target_person_id", bytes(target_person_id)),
            ("org_source_type", org_source_type.into()),
        ],
    )?;

    let row = db
        .query_one(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            values,
        ))
        .await?;
    Ok(row.is_some())
}

/// Depth-bounded subtree rooted at `root_person_id`. Ported verbatim from
/// `SqlSubchart.GetSubchart`: a recursive descent over `org_chart` (anchor =
/// the root with a NULL parent) joined to a `ROW_NUMBER()` latest-observation
/// pass. `max_depth = None` = unbounded (bounded by MariaDB's
/// `cte_max_recursion_depth`). The caller gates visibility on the root before
/// calling this (see [`is_target_in_visible_set`]).
///
/// # Errors
///
/// Returns an error if the query fails or a stored id is not 16 bytes.
pub async fn get_subchart_flat(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    root_person_id: Uuid,
    source_type: &str,
    max_depth: Option<i32>,
    valid_at: Option<DateTime>,
) -> anyhow::Result<Vec<SubchartFlatNode>> {
    const SQL: &str = r"
        WITH RECURSIVE
        subtree (person_id, parent_person_id, depth) AS (
            SELECT @root_person_id, CAST(NULL AS BINARY(16)), 0
            UNION ALL
            SELECT oc.child_person_id, oc.parent_person_id, s.depth + 1
            FROM subtree s
            JOIN org_chart oc
              ON  oc.insight_tenant_id   = @tenant_id
              AND oc.parent_person_id    = s.person_id
              AND oc.insight_source_type = @source_type
              AND oc.valid_from <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
              AND (oc.valid_to IS NULL OR oc.valid_to > COALESCE(@valid_at, UTC_TIMESTAMP(6)))
            WHERE @max_depth IS NULL OR s.depth < @max_depth
        ),
        latest_obs AS (
            SELECT
                p.person_id,
                p.value_type,
                COALESCE(p.value_id, p.value_full_text) AS value_,
                ROW_NUMBER() OVER (
                    PARTITION BY p.person_id, p.value_type
                    ORDER BY p.created_at DESC
                ) AS rn
            FROM persons p
            WHERE p.insight_tenant_id = @tenant_id
              AND p.person_id IN (SELECT person_id FROM subtree)
              AND p.value_type IN ('email', 'display_name', 'job_title', 'status')
              AND p.created_at <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
        )
        SELECT
            s.person_id,
            s.parent_person_id,
            s.depth,
            MAX(CASE WHEN l.value_type = 'email'        THEN l.value_ END) AS email,
            MAX(CASE WHEN l.value_type = 'display_name' THEN l.value_ END) AS display_name,
            MAX(CASE WHEN l.value_type = 'job_title'    THEN l.value_ END) AS job_title,
            MAX(CASE WHEN l.value_type = 'status'       THEN l.value_ END) AS status
        FROM subtree s
        LEFT JOIN latest_obs l
          ON l.person_id = s.person_id AND l.rn = 1
        GROUP BY s.person_id, s.parent_person_id, s.depth
        ORDER BY s.depth, s.person_id
    ";

    let (sql, values) = bind_named(
        SQL,
        &[
            ("root_person_id", bytes(root_person_id)),
            ("tenant_id", bytes(tenant_id)),
            ("source_type", source_type.into()),
            ("valid_at", valid_at.into()),
            ("max_depth", max_depth.into()),
        ],
    )?;

    let rows = db
        .query_all(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            values,
        ))
        .await?;
    rows.iter().map(row_to_flat).collect()
}

/// Forest variant: every root the `viewer_person_id` can see, one subtree per
/// visible top of the source's org chart. Ported verbatim from
/// `SqlSubchart.GetForest` (`visible_set` → `in_source` → `roots` → `subtree` →
/// `latest_obs`). Roots surface with `parent_person_id IS NULL` regardless of
/// their stored row; singleton orphans (no children in the source) are dropped
/// by the `roots` CTE's `EXISTS` guard.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id is not 16 bytes.
#[allow(clippy::too_many_lines)] // one verbatim multi-CTE SQL const dominates
pub async fn get_forest_flat(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    viewer_person_id: Uuid,
    source_type: &str,
    max_depth: Option<i32>,
    valid_at: Option<DateTime>,
) -> anyhow::Result<Vec<SubchartFlatNode>> {
    const SQL: &str = r"
        WITH RECURSIVE
        visible_set (person_id) AS (
            SELECT @viewer_person_id
            UNION
            SELECT viewed_person_id
            FROM visibility
            WHERE insight_tenant_id = @tenant_id
              AND viewer_person_id  = @viewer_person_id
              AND viewed_person_id  IS NOT NULL
              AND valid_from <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
              AND (valid_to IS NULL OR valid_to > COALESCE(@valid_at, UTC_TIMESTAMP(6)))
            UNION
            SELECT DISTINCT person_id FROM persons
            WHERE insight_tenant_id = @tenant_id
              AND EXISTS (
                  SELECT 1 FROM visibility
                  WHERE insight_tenant_id = @tenant_id
                    AND viewer_person_id  = @viewer_person_id
                    AND viewed_person_id  IS NULL
                    AND valid_from <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
                    AND (valid_to IS NULL OR valid_to > COALESCE(@valid_at, UTC_TIMESTAMP(6)))
              )
            UNION
            SELECT oc.child_person_id
            FROM visible_set vs
            JOIN org_chart oc
              ON  oc.parent_person_id    = vs.person_id
              AND oc.insight_tenant_id   = @tenant_id
              AND oc.insight_source_type = @source_type
              AND oc.valid_from <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
              AND (oc.valid_to IS NULL OR oc.valid_to > COALESCE(@valid_at, UTC_TIMESTAMP(6)))
        ),
        in_source AS (
            SELECT DISTINCT vs.person_id
            FROM visible_set vs
            JOIN org_chart oc
              ON  oc.child_person_id     = vs.person_id
              AND oc.insight_tenant_id   = @tenant_id
              AND oc.insight_source_type = @source_type
              AND oc.valid_from <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
              AND (oc.valid_to IS NULL OR oc.valid_to > COALESCE(@valid_at, UTC_TIMESTAMP(6)))
        ),
        roots AS (
            SELECT DISTINCT i.person_id
            FROM in_source i
            JOIN org_chart oc
              ON  oc.child_person_id     = i.person_id
              AND oc.insight_tenant_id   = @tenant_id
              AND oc.insight_source_type = @source_type
              AND oc.valid_from <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
              AND (oc.valid_to IS NULL OR oc.valid_to > COALESCE(@valid_at, UTC_TIMESTAMP(6)))
            WHERE (oc.parent_person_id IS NULL
                OR NOT EXISTS (
                    SELECT 1 FROM in_source i2
                    WHERE i2.person_id = oc.parent_person_id
                ))
              AND EXISTS (
                  SELECT 1 FROM org_chart c2
                  WHERE c2.parent_person_id    = i.person_id
                    AND c2.insight_tenant_id   = @tenant_id
                    AND c2.insight_source_type = @source_type
                    AND c2.valid_from <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
                    AND (c2.valid_to IS NULL OR c2.valid_to > COALESCE(@valid_at, UTC_TIMESTAMP(6)))
              )
        ),
        subtree (person_id, parent_person_id, depth) AS (
            SELECT person_id, CAST(NULL AS BINARY(16)), 0 FROM roots
            UNION ALL
            SELECT oc.child_person_id, oc.parent_person_id, s.depth + 1
            FROM subtree s
            JOIN org_chart oc
              ON  oc.parent_person_id    = s.person_id
              AND oc.insight_tenant_id   = @tenant_id
              AND oc.insight_source_type = @source_type
              AND oc.valid_from <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
              AND (oc.valid_to IS NULL OR oc.valid_to > COALESCE(@valid_at, UTC_TIMESTAMP(6)))
            WHERE @max_depth IS NULL OR s.depth < @max_depth
        ),
        latest_obs AS (
            SELECT
                p.person_id,
                p.value_type,
                COALESCE(p.value_id, p.value_full_text) AS value_,
                ROW_NUMBER() OVER (
                    PARTITION BY p.person_id, p.value_type
                    ORDER BY p.created_at DESC
                ) AS rn
            FROM persons p
            WHERE p.insight_tenant_id = @tenant_id
              AND p.person_id IN (SELECT person_id FROM subtree)
              AND p.value_type IN ('email', 'display_name', 'job_title', 'status')
              AND p.created_at <= COALESCE(@valid_at, UTC_TIMESTAMP(6))
        )
        SELECT
            s.person_id,
            s.parent_person_id,
            s.depth,
            MAX(CASE WHEN l.value_type = 'email'        THEN l.value_ END) AS email,
            MAX(CASE WHEN l.value_type = 'display_name' THEN l.value_ END) AS display_name,
            MAX(CASE WHEN l.value_type = 'job_title'    THEN l.value_ END) AS job_title,
            MAX(CASE WHEN l.value_type = 'status'       THEN l.value_ END) AS status
        FROM subtree s
        LEFT JOIN latest_obs l
          ON l.person_id = s.person_id AND l.rn = 1
        GROUP BY s.person_id, s.parent_person_id, s.depth
        ORDER BY s.depth, s.person_id
    ";

    let (sql, values) = bind_named(
        SQL,
        &[
            ("viewer_person_id", bytes(viewer_person_id)),
            ("tenant_id", bytes(tenant_id)),
            ("valid_at", valid_at.into()),
            ("source_type", source_type.into()),
            ("max_depth", max_depth.into()),
        ],
    )?;

    let rows = db
        .query_all(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            values,
        ))
        .await?;
    rows.iter().map(row_to_flat).collect()
}

/// UUID → big-endian `BINARY(16)` bound value (matches the .NET
/// `ToByteArray(bigEndian: true)`).
fn bytes(id: Uuid) -> Value {
    id.as_bytes().to_vec().into()
}
