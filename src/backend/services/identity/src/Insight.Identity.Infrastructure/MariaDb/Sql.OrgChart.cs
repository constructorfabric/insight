namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// SQL statements against <c>org_chart</c>, the SCD2 cache of
/// parent->child edges derived from <c>persons</c> observations with
/// <c>value_type='parent_person_id'</c>. Phase 1 of
/// cyberfabric/cyber-insight#348.
///
/// All queries read CURRENT edges only (<c>valid_to IS NULL</c>).
/// Temporal as-of queries (Phase 3+) will use a different statement
/// with <c>valid_from &lt;= @as_of AND (valid_to IS NULL OR valid_to &gt; @as_of)</c>
/// and the <c>idx_valid_from</c> index — kept out of this file until
/// there is a caller.
///
/// The rebuild SQL that populates the table lives in the Python seeder
/// (<c>seed-persons-from-identity-input.py</c> step 9) and is NOT in
/// this file because the service does not own the rebuild path — it
/// only reads the materialized result.
/// </summary>
internal static class SqlOrgChart
{
    /// <summary>
    /// Current parent edges for one child, across every source instance
    /// that has a parent observation. Phase 1 invariant: at most one
    /// CURRENT parent per (tenant, source_type, source_id, child),
    /// enforced by the <c>idx_current_parent</c> index shape.
    /// </summary>
    public const string CurrentParentsForChild = """
        SELECT
            insight_source_type,
            insight_source_id,
            child_person_id,
            parent_person_id,
            valid_from
        FROM org_chart
        WHERE insight_tenant_id = @tenant_id
          AND child_person_id   = @child_person_id
          AND valid_to IS NULL
        ORDER BY insight_source_type, insight_source_id
        """;

    /// <summary>
    /// Current direct-children edges for one parent, across every
    /// source instance that recorded the relationship. Hot path for
    /// the Phase-2 subordinates field and the Phase-3 subchart endpoint.
    /// </summary>
    public const string CurrentChildrenForParent = """
        SELECT
            insight_source_type,
            insight_source_id,
            child_person_id,
            parent_person_id,
            valid_from
        FROM org_chart
        WHERE insight_tenant_id  = @tenant_id
          AND parent_person_id   = @parent_person_id
          AND valid_to IS NULL
        ORDER BY insight_source_type, insight_source_id, child_person_id
        """;
}
