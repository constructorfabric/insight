namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// SQL for the `visibility` table. `ActiveGrantsByViewer` fetches a
/// viewer's active grants; `IsTargetInVisibleSet` answers the
/// can-A-see-B predicate via a single recursive CTE that unions
/// viewer + active grant targets + (target itself when a whole-tenant
/// grant exists) and walks `org_chart` descents filtered to the
/// configured source type.
/// </summary>
internal static class SqlVisibility
{
    public const string ActiveGrantsByViewer = """
        SELECT visibility_id, insight_tenant_id, viewer_person_id, viewed_person_id,
               valid_from, valid_to, author_person_id, reason, created_at
        FROM visibility
        WHERE insight_tenant_id = @tenant_id
          AND viewer_person_id  = @viewer_person_id
          AND valid_to IS NULL
        """;

    public const string IsTargetInVisibleSet = """
        WITH RECURSIVE visible_set (person_id) AS (
            SELECT @viewer_person_id
            UNION
            SELECT viewed_person_id
            FROM visibility
            WHERE insight_tenant_id = @tenant_id
              AND viewer_person_id  = @viewer_person_id
              AND viewed_person_id  IS NOT NULL
              AND valid_to IS NULL
            UNION
            SELECT @target_person_id
            WHERE EXISTS (
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
              AND oc.insight_source_type = @org_source_type
              AND oc.valid_to IS NULL
        )
        SELECT EXISTS (SELECT 1 FROM visible_set WHERE person_id = @target_person_id)
        """;

    private const string ColumnList =
        "visibility_id, insight_tenant_id, viewer_person_id, viewed_person_id, " +
        "valid_from, valid_to, author_person_id, reason, created_at";

    public const string GetById = $"""
        SELECT {ColumnList}
        FROM visibility
        WHERE insight_tenant_id = @tenant_id
          AND visibility_id     = @visibility_id
        LIMIT 1
        """;

    public const string ListBase = $"""
        SELECT {ColumnList}
        FROM visibility
        WHERE insight_tenant_id = @tenant_id
        """;

    public const string Insert = """
        INSERT INTO visibility
            (visibility_id, insight_tenant_id, viewer_person_id, viewed_person_id,
             valid_from, valid_to, author_person_id, reason)
        VALUES
            (@visibility_id, @tenant_id, @viewer_person_id, @viewed_person_id,
             IFNULL(@valid_from, UTC_TIMESTAMP(6)), NULL, @author_person_id, @reason)
        """;

    public const string SoftDelete = """
        UPDATE visibility
        SET valid_to = UTC_TIMESTAMP(6),
            reason   = COALESCE(@reason, reason)
        WHERE insight_tenant_id = @tenant_id
          AND visibility_id     = @visibility_id
          AND valid_to IS NULL
        """;
}
