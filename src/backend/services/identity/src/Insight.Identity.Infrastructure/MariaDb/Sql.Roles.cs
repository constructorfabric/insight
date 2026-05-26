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

    public const string RoleById = """
        SELECT role_id, name
        FROM roles
        WHERE role_id = @role_id
        LIMIT 1
        """;

    public const string InsertRole = """
        INSERT INTO roles (role_id, name)
        VALUES (@role_id, @name)
        """;

    // Atomic delete-if-unused: refuse if any active `person_roles` row
    // references the role (any tenant). One round-trip — no separate
    // COUNT call, so no TOCTOU race between guard and write. Disambiguate
    // rows_affected==0 in the caller via a second read.
    public const string TryDeleteRoleIfUnused = """
        DELETE FROM roles
        WHERE role_id = @role_id
          AND NOT EXISTS (
              SELECT 1 FROM person_roles
              WHERE role_id = @role_id AND valid_to IS NULL
          )
        """;

    public const string CountActivePersonRolesByRole = """
        SELECT COUNT(*)
        FROM person_roles
        WHERE insight_tenant_id = @tenant_id
          AND role_id           = @role_id
          AND valid_to IS NULL
        """;

    public const string CountActivePersonRolesByRoleAnyTenant = """
        SELECT COUNT(*)
        FROM person_roles
        WHERE role_id    = @role_id
          AND valid_to IS NULL
        """;

    private const string PersonRoleColumnList =
        "person_role_id, insight_tenant_id, person_id, role_id, " +
        "valid_from, valid_to, author_person_id, reason, created_at";

    public const string PersonRoleById = $"""
        SELECT {PersonRoleColumnList}
        FROM person_roles
        WHERE insight_tenant_id = @tenant_id
          AND person_role_id    = @person_role_id
        LIMIT 1
        """;

    public const string PersonRoleListBase = $"""
        SELECT {PersonRoleColumnList}
        FROM person_roles
        WHERE insight_tenant_id = @tenant_id
        """;

    public const string InsertPersonRole = """
        INSERT INTO person_roles
            (person_role_id, insight_tenant_id, person_id, role_id,
             valid_from, valid_to, author_person_id, reason)
        VALUES
            (@person_role_id, @tenant_id, @person_id, @role_id,
             IFNULL(@valid_from, UTC_TIMESTAMP(6)), NULL, @author_person_id, @reason)
        """;

    // Soft-delete with last-admin protection. Runs as the second
    // statement inside a transaction; the first statement is
    // <see cref="LockActiveAdminsInTenantForUpdate"/>, which pins
    // every active admin row in the target's tenant under InnoDB
    // row-level write locks. With those locks held, no concurrent
    // transaction can revoke another admin between our COUNT and our
    // UPDATE — TOCTOU is closed even when the correlated COUNT inside
    // the derived table is treated as a snapshot read.
    //
    // The UPDATE itself still uses the derived-table `row_with_count`
    // pattern to materialise tenant + active-admin count once, then
    // applies the guard `role_id <> admin OR count > 1`. Caller
    // disambiguates rows_affected==0 between 404 (row gone or already
    // revoked) and 422 last_admin_protected via a second read.
    public const string TrySoftDeletePersonRoleProtectingLastAdmin = """
        UPDATE person_roles AS target
        JOIN (
            SELECT
                pr.person_role_id,
                pr.role_id,
                (
                    SELECT COUNT(*)
                    FROM person_roles AS adm
                    WHERE adm.insight_tenant_id = pr.insight_tenant_id
                      AND adm.role_id           = @admin_role_id
                      AND adm.valid_to IS NULL
                ) AS active_admin_cnt
            FROM person_roles AS pr
            WHERE pr.person_role_id = @person_role_id
              AND pr.valid_to IS NULL
        ) AS row_with_count
          ON row_with_count.person_role_id = target.person_role_id
        SET target.valid_to = UTC_TIMESTAMP(6),
            target.reason   = COALESCE(@reason, target.reason)
        WHERE target.valid_to IS NULL
          AND (
              row_with_count.role_id <> @admin_role_id
              OR row_with_count.active_admin_cnt > 1
          )
        """;

    // Pin-the-count: row-level X-locks on every active admin row in the
    // target's tenant. The aggregate COUNT(*) scans the same rows
    // FOR UPDATE pins, so the read serialises with itself across
    // concurrent transactions. Issued as the first statement inside the
    // soft-delete transaction; followed by
    // <see cref="TrySoftDeletePersonRoleProtectingLastAdmin"/>.
    //
    // We resolve the target's tenant via a scalar subquery on
    // person_role_id rather than passing it as a parameter — the C#
    // caller does not always know the tenant up-front and we want a
    // single round-trip for the lock acquisition.
    public const string LockActiveAdminsInTenantForUpdate = """
        SELECT COUNT(*)
        FROM person_roles
        WHERE insight_tenant_id = (
                SELECT insight_tenant_id FROM person_roles
                WHERE person_role_id = @person_role_id
              )
          AND role_id  = @admin_role_id
          AND valid_to IS NULL
        FOR UPDATE
        """;
}
