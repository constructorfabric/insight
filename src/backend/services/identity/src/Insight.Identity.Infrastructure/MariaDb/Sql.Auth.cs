namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// SQL for caller resolution at request time (#346 follow-up). The
/// account-id lookup is the new path; the email path reuses
/// <see cref="SqlProfiles.ResolvePersonIdsByEmail"/>. Both stay
/// tenant-scoped — every query in the service filters by
/// <c>insight_tenant_id</c>.
/// </summary>
internal static class SqlAuth
{
    /// <summary>
    /// Returns the <c>person_id</c> bound to a source-native account id
    /// within the tenant (active row only). Used to map a JWT
    /// <c>oid</c> or <c>sub</c> claim to a person. Uses
    /// <c>idx_by_account</c>. The <c>ORDER BY valid_from DESC LIMIT 1</c>
    /// keeps the result deterministic if the table ever holds more than
    /// one active row for the same account.
    /// </summary>
    public const string ResolvePersonIdByAccountId = """
        SELECT person_id
        FROM account_person_map
        WHERE insight_tenant_id = @tenant_id
          AND source_account_id = @account_id
          AND valid_to IS NULL
        ORDER BY valid_from DESC
        LIMIT 1
        """;
}
