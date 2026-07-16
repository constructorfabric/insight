namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// Centralised SQL statements for the <c>persons</c> table. The
/// <c>ROW_NUMBER() OVER PARTITION BY</c> pattern selects the latest
/// observation per <c>(insight_source_type, insight_source_id, value_type)</c>
/// for a given <c>(tenant, person)</c>; that is the canonical
/// "latest-per-source" projection ADR-0003 mandates.
/// </summary>
internal static class Sql
{
    /// <summary>
    /// Resolve <c>person_id</c> from a lookup email. Picks the latest
    /// observation across all sources that maps the email back to a
    /// person within the tenant; if the email was rebound to a new
    /// person on any source, the old email returns no row.
    /// </summary>
    public const string ResolvePersonIdByEmail = """
        WITH ranked AS (
            SELECT
                person_id,
                id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, value_type, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn,
                created_at
            FROM persons
            WHERE insight_tenant_id = @tenant_id
              AND value_type = 'email'
              AND value_id = @email
        )
        SELECT person_id
        FROM ranked
        WHERE rn = 1
        ORDER BY created_at DESC, id DESC
        LIMIT 1
        """;

    /// <summary>
    /// Tenant-AGNOSTIC email → person_id resolution for the login bootstrap
    /// (the authenticator's service-only `/internal/persons/by-email` call): at
    /// login the caller's tenant is not yet known, so the tenant filter is
    /// dropped. Any matching tenant's latest observation wins.
    /// </summary>
    public const string ResolvePersonIdByEmailAnyTenant = """
        WITH ranked AS (
            SELECT
                person_id,
                id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, value_type, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn,
                created_at
            FROM persons
            WHERE value_type = 'email'
              AND value_id = @email
        )
        SELECT person_id
        FROM ranked
        WHERE rn = 1
        ORDER BY created_at DESC, id DESC
        LIMIT 1
        """;

    /// <summary>
    /// Latest observation per <c>(insight_source_type, insight_source_id,
    /// value_type)</c> for a single person within the tenant.
    /// <c>value_effective</c> is the generated coalesce of the three
    /// storage columns and is the one the assembler reads.
    /// </summary>
    public const string LatestObservationsForPerson = """
        WITH ranked AS (
            SELECT
                person_id,
                insight_source_type,
                insight_source_id,
                value_type,
                value_effective,
                created_at,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id = @tenant_id
              AND person_id = @person_id
        )
        SELECT person_id, insight_source_type, insight_source_id, value_type, value_effective, created_at
        FROM ranked
        WHERE rn = 1
        """;

    public const string Healthcheck = "SELECT 1";
}
