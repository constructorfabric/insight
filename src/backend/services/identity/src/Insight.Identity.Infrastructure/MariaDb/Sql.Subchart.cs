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
    /// <summary>
    /// Forest variant (#344 follow-up). Same shape as
    /// <see cref="GetSubchart"/> but seeded from EVERY visible root in
    /// the caller's <c>visible_set</c> instead of a single
    /// <c>@root_person_id</c>. Implementation glues four CTEs together:
    /// <list type="number">
    ///   <item><c>visible_set</c> — viewer + explicit grants + (wildcard
    ///   expansion to all persons in tenant) + org_chart descent.</item>
    ///   <item><c>in_source</c> — visible persons that have a current
    ///   <c>org_chart</c> row for <c>@source_type</c> (path-B membership).</item>
    ///   <item><c>roots</c> — visible-in-source persons whose current
    ///   parent is NULL or invisible (the caller's view of the forest's
    ///   topmost rows).</item>
    ///   <item><c>subtree</c> — depth-bounded descent from all roots in
    ///   one shot.</item>
    /// </list>
    /// Parameters: <c>@tenant_id</c>, <c>@viewer_person_id</c>,
    /// <c>@source_type</c>, <c>@max_depth</c>. Result columns mirror
    /// <see cref="GetSubchart"/>; ROOTS surface with
    /// <c>parent_person_id IS NULL</c> regardless of their actual
    /// org_chart row, so the service layer can group by parent.
    /// </summary>
    public const string GetForest = """
        WITH RECURSIVE
        visible_set (person_id) AS (
            SELECT @viewer_person_id
            UNION
            SELECT viewed_person_id
            FROM visibility
            WHERE insight_tenant_id = @tenant_id
              AND viewer_person_id  = @viewer_person_id
              AND viewed_person_id  IS NOT NULL
              AND valid_to IS NULL
            UNION
            -- Wildcard grant (viewed_person_id IS NULL) expands the
            -- visible_set to every person in the tenant — caller is a
            -- visibility super-user. Note: tenant-wide, not source-
            -- scoped, by team decision.
            SELECT DISTINCT person_id FROM persons
            WHERE insight_tenant_id = @tenant_id
              AND EXISTS (
                  SELECT 1 FROM visibility
                  WHERE insight_tenant_id = @tenant_id
                    AND viewer_person_id  = @viewer_person_id
                    AND viewed_person_id  IS NULL
                    AND valid_to IS NULL
              )
            UNION
            SELECT oc.child_person_id
            FROM visible_set vs
            JOIN org_chart oc
              ON  oc.parent_person_id    = vs.person_id
              AND oc.insight_tenant_id   = @tenant_id
              AND oc.insight_source_type = @source_type
              AND oc.valid_to IS NULL
        ),
        in_source AS (
            -- Visible persons that have a CURRENT org_chart row for the
            -- requested source (under path B every member of the source
            -- has one — with parent or with NULL parent).
            SELECT DISTINCT vs.person_id
            FROM visible_set vs
            JOIN org_chart oc
              ON  oc.child_person_id     = vs.person_id
              AND oc.insight_tenant_id   = @tenant_id
              AND oc.insight_source_type = @source_type
              AND oc.valid_to IS NULL
        ),
        roots AS (
            -- A root in the caller's view: a visible-in-source person
            -- whose current parent is NULL (true top) or invisible to
            -- the caller (their actual parent is outside visible_set).
            -- Orphan filter (#344): require at least one current child
            -- in the same source — singleton "trees" (no parent + no
            -- children) are dropped here so the endpoint does not
            -- surface them to clients. Applied at SQL so depth=0 still
            -- returns real tops with an empty subordinates array.
            SELECT DISTINCT i.person_id
            FROM in_source i
            JOIN org_chart oc
              ON  oc.child_person_id     = i.person_id
              AND oc.insight_tenant_id   = @tenant_id
              AND oc.insight_source_type = @source_type
              AND oc.valid_to IS NULL
            WHERE (oc.parent_person_id IS NULL
                OR oc.parent_person_id NOT IN (SELECT person_id FROM in_source))
              AND EXISTS (
                  SELECT 1 FROM org_chart c2
                  WHERE c2.parent_person_id    = i.person_id
                    AND c2.insight_tenant_id   = @tenant_id
                    AND c2.insight_source_type = @source_type
                    AND c2.valid_to IS NULL
              )
        ),
        subtree (person_id, parent_person_id, depth) AS (
            -- Anchor: all roots in one shot, parent_person_id = NULL
            -- so the service layer groups them as the forest tops.
            SELECT person_id, CAST(NULL AS BINARY(16)), 0 FROM roots
            UNION ALL
            SELECT oc.child_person_id, oc.parent_person_id, s.depth + 1
            FROM subtree s
            JOIN org_chart oc
              ON  oc.parent_person_id    = s.person_id
              AND oc.insight_tenant_id   = @tenant_id
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
