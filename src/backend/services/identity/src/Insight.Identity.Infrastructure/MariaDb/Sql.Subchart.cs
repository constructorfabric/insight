namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// SQL for the depth-bounded org subchart endpoint (#348 Phase 3).
/// Single recursive CTE over <c>org_chart</c> for the subtree
/// traversal, plus a derived window-function CTE to pick the latest
/// observation per (person, value_type) for the response fields.
/// Returns a flat row set ordered by depth; the service layer
/// assembles the tree in C#.
/// </summary>
internal static class SqlSubchart
{
    /// <summary>
    /// Parameters:
    /// <list type="bullet">
    ///   <item><c>@tenant_id</c> — BINARY(16) big-endian.</item>
    ///   <item><c>@root_person_id</c> — BINARY(16) big-endian.</item>
    ///   <item><c>@source_type</c> — string (e.g. <c>bamboohr</c>).</item>
    ///   <item><c>@max_depth</c> — int or NULL. NULL = unbounded
    ///   (constrained by MariaDB's <c>cte_max_recursion_depth</c>).</item>
    /// </list>
    /// Result columns: <c>person_id</c>, <c>parent_person_id</c>
    /// (NULL on root), <c>depth</c>, <c>email</c>, <c>display_name</c>,
    /// <c>job_title</c>, <c>status</c> (each text field may be NULL when
    /// no observation of that type exists).
    /// </summary>
    public const string GetSubchart = """
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
              AND oc.valid_to IS NULL
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
        """;
}
