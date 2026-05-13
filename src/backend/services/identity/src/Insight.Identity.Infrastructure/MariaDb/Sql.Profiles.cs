namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// SQL for the Phase-2 <c>POST /v1/profiles</c> endpoint
/// (cyberfabric/cyber-insight#347).
///
/// All three queries share the canonical partition key per ADR-0003 +
/// the identity-resolution data-model spec — the latest observation per
/// <c>(insight_tenant_id, person_id, insight_source_type, insight_source_id,
/// value_type)</c>. A newer observation on the same partition fully
/// supersedes older ones; the data invariant downstream is that there
/// is at most one current <c>person_id</c> matching a given (email) or
/// (source_type, source_id, source-native id).
///
/// Note: the Phase-1 queries in <see cref="Sql"/> use a different
/// partition (with <c>value_id</c> inside it, without <c>person_id</c>),
/// which fits the GET-by-email contract preserved on <c>/v1/persons/{email}</c>.
/// We intentionally do not change Phase-1 SQL here.
/// </summary>
internal static class SqlProfiles
{
    /// <summary>
    /// Resolve the set of <c>person_id</c>s whose CURRENT email (latest
    /// per source-instance) equals <c>@value</c> within the tenant.
    /// Caller fails with 422 ambiguous_profile if the row count is > 1
    /// and 404 person_not_found if 0.
    /// </summary>
    public const string ResolvePersonIdsByEmail = """
        WITH ranked AS (
            SELECT
                person_id,
                value_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id = @tenant_id
              AND value_type = 'email'
        )
        SELECT DISTINCT person_id
        FROM ranked
        WHERE rn = 1
          AND value_id = @value
        """;

    /// <summary>
    /// Resolve the set of <c>person_id</c>s whose CURRENT
    /// <c>value_type='id'</c> observation on the given source instance
    /// equals <c>@value</c>. Source-instance scoped: the (source_type,
    /// source_id) pair is part of the WHERE clause, not the search
    /// value.
    /// </summary>
    public const string ResolvePersonIdsBySourceId = """
        WITH ranked AS (
            SELECT
                person_id,
                value_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id   = @tenant_id
              AND insight_source_type = @source_type
              AND insight_source_id   = @source_id
              AND value_type          = 'id'
        )
        SELECT DISTINCT person_id
        FROM ranked
        WHERE rn = 1
          AND value_id = @value
        """;

    /// <summary>
    /// All current source-native ids for one person, one row per source
    /// instance (latest <c>value_type='id'</c> per (source_type,
    /// source_id) partition). Used to build the <c>ids[]</c> list in the
    /// response.
    /// </summary>
    public const string CurrentSourceIdsForPerson = """
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
            WHERE insight_tenant_id = @tenant_id
              AND person_id         = @person_id
              AND value_type        = 'id'
        )
        SELECT insight_source_type, insight_source_id, value_id AS value
        FROM ranked
        WHERE rn = 1
        ORDER BY insight_source_type, insight_source_id
        """;
}
