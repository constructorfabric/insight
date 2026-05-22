namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// SQL for the `roles` and `person_roles` tables (#346 step 1).
/// `roles` is global (no tenant column); `person_roles` is per-tenant.
/// </summary>
internal static class SqlRoles
{
    public const string RoleByName = """
        SELECT role_id, name
        FROM roles
        WHERE name = @name
        LIMIT 1
        """;

    public const string ListAllRoles = """
        SELECT role_id, name
        FROM roles
        ORDER BY name
        """;

    public const string HasActivePersonRole = """
        SELECT EXISTS (
            SELECT 1
            FROM person_roles
            WHERE insight_tenant_id = @tenant_id
              AND person_id         = @person_id
              AND role_id           = @role_id
              AND valid_to IS NULL
        )
        """;

    public const string ActivePersonRolesByPerson = """
        SELECT person_role_id, insight_tenant_id, person_id, role_id,
               valid_from, valid_to, author_person_id, reason, created_at
        FROM person_roles
        WHERE insight_tenant_id = @tenant_id
          AND person_id         = @person_id
          AND valid_to IS NULL
        """;
}
