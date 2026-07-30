using Insight.Identity.Domain.Services;
using MySqlConnector;
using Testcontainers.MariaDb;
using Xunit;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// Spins up a single MariaDB container per xUnit collection and applies
/// the persons schema. Mirrors the canonical DDL from
/// <c>src/backend/services/identity/src/migration/m20260421_000001_persons.rs</c>.
/// </summary>
public sealed class MariaDbFixture : IAsyncLifetime
{
    private readonly MariaDbContainer _container = new MariaDbBuilder("mariadb:11.4")
        .WithDatabase("identity")
        .WithUsername("insight")
        .WithPassword("insight-pass")
        .Build();

    public string ConnectionString => _container.GetConnectionString();

    public string ConnectionUrl
    {
        get
        {
            var b = new MySqlConnectionStringBuilder(ConnectionString);
            return $"mysql://{Uri.EscapeDataString(b.UserID)}:{Uri.EscapeDataString(b.Password)}@{b.Server}:{b.Port}/{b.Database}";
        }
    }

    public async Task InitializeAsync()
    {
        await _container.StartAsync().ConfigureAwait(false);
        await ApplySchemaAsync().ConfigureAwait(false);
    }

    public Task DisposeAsync() => _container.DisposeAsync().AsTask();

    /// <summary>
    /// Insert a whole-tenant visibility grant (viewer = given person,
    /// viewed_person_id IS NULL) so the caller can see every row in
    /// the tenant. Called from endpoint-test InitializeAsync after
    /// <see cref="ResetAsync"/> so existing happy-path scenarios still
    /// pass through the visibility gate that #346 step 3 added.
    /// </summary>
    public async Task SeedWholeTenantVisibilityAsync(Guid tenantId, Guid viewerPersonId)
    {
        await using var conn = new MySqlConnection(ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            INSERT INTO visibility
                (visibility_id, insight_tenant_id, viewer_person_id, viewed_person_id,
                 valid_from, valid_to, author_person_id, reason)
            VALUES (@id, @tenant, @viewer, NULL, '2020-01-01 00:00:00', NULL, @viewer, NULL)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@id",     Guid.NewGuid().ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@viewer", viewerPersonId.ToByteArray(bigEndian: true));
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    public async Task ResetAsync()
    {
        await using var conn = new MySqlConnection(ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        // `org_chart` references no tenant of its own — its rows
        // are derived from persons by the seeder, so clearing both keeps
        // each test starting from an empty graph regardless of what the
        // previous test inserted.
        await using (var cmd = new MySqlCommand("DELETE FROM org_chart", conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand("DELETE FROM persons", conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        // #346 step-1 OrgChart Visibility tables. `roles` is mostly NOT cleared — the
        // admin seed row is part of the schema-bootstrap contract and
        // every test depends on it being there — but any ad-hoc role a
        // multi-role test inserts is wiped so it doesn't bleed into
        // siblings. Per-test rows in `visibility` and `person_roles`
        // are always cleared.
        await using (var cmd = new MySqlCommand("DELETE FROM visibility", conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand("DELETE FROM person_roles", conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand("DELETE FROM account_person_map", conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand("DELETE FROM operations", conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand(
            "DELETE FROM roles WHERE role_id <> @admin_id",
            conn))
        {
            cmd.Parameters.AddWithValue("@admin_id", Roles.Admin.ToByteArray(bigEndian: true));
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        }
    }

    private async Task ApplySchemaAsync()
    {
        await using var conn = new MySqlConnection(ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand(PersonsDdl, conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand(OrgChartDdl, conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand(VisibilityDdl, conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand(RolesDdl, conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand(PersonRolesDdl, conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand(AccountPersonMapDdl, conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand(OperationsDdl, conn))
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        await using (var cmd = new MySqlCommand(AdminRoleSeed, conn))
        {
            cmd.Parameters.AddWithValue("@admin_id", Roles.Admin.ToByteArray(bigEndian: true));
            await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
        }
    }

    private const string PersonsDdl = """
        CREATE TABLE IF NOT EXISTS persons (
            id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
            value_type VARCHAR(50) NOT NULL,
            insight_source_type VARCHAR(30) NOT NULL,
            insight_source_id BINARY(16) NOT NULL,
            insight_tenant_id BINARY(16) NOT NULL,
            value_id VARCHAR(320) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL,
            value_full_text VARCHAR(512) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL,
            value TEXT NULL,
            value_effective TEXT
                GENERATED ALWAYS AS (COALESCE(value_id, value_full_text, value)) STORED,
            value_hash CHAR(64) CHARACTER SET ascii COLLATE ascii_bin
                GENERATED ALWAYS AS (SHA2(COALESCE(value_id, value_full_text, value), 256)) STORED,
            person_id BINARY(16) NOT NULL,
            author_person_id BINARY(16) NOT NULL,
            reason TEXT NULL,
            created_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6)),
            UNIQUE KEY uq_person_observation (
                insight_tenant_id, person_id, insight_source_type, insight_source_id,
                value_type, created_at
            ),
            INDEX idx_value_id (insight_tenant_id, value_type, value_id),
            INDEX idx_value_full_text (insight_tenant_id, value_type, value_full_text),
            INDEX idx_person_id (person_id),
            INDEX idx_tenant_person (insight_tenant_id, person_id),
            INDEX idx_source (insight_source_type, insight_source_id)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
        """;

    // Mirrors of Migrations/006_visibility.sql + 007_roles.sql + 008_person_roles.sql.
    // Kept inline so the fixture stays self-contained (no DbUp at test start).
    private const string VisibilityDdl = """
        CREATE TABLE IF NOT EXISTS visibility (
            visibility_id     BINARY(16) NOT NULL,
            insight_tenant_id BINARY(16) NOT NULL,
            viewer_person_id  BINARY(16) NOT NULL,
            viewed_person_id  BINARY(16) NULL,
            valid_from        DATETIME(6) NOT NULL,
            valid_to          DATETIME(6) NULL,
            author_person_id  BINARY(16) NOT NULL,
            reason            VARCHAR(500) NULL,
            created_at        DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6)),
            PRIMARY KEY (visibility_id),
            CONSTRAINT chk_visibility_interval
                CHECK (valid_to IS NULL OR valid_from <= valid_to),
            INDEX idx_viewer_current
                (insight_tenant_id, viewer_person_id, valid_to, viewed_person_id)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
        """;

    private const string RolesDdl = """
        CREATE TABLE IF NOT EXISTS roles (
            role_id BINARY(16) NOT NULL,
            name    VARCHAR(64) NOT NULL,
            PRIMARY KEY (role_id),
            UNIQUE KEY uk_name (name)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
        """;

    // Mirror of Migrations/011_operations.sql.
    private const string OperationsDdl = """
        CREATE TABLE IF NOT EXISTS operations (
            operation_id      BINARY(16)   NOT NULL,
            operation_type    VARCHAR(64)  NOT NULL,
            status            VARCHAR(16)  NOT NULL,
            insight_tenant_id BINARY(16)   NOT NULL,
            author_person_id  BINARY(16)   NOT NULL,
            request_json      JSON         NULL,
            summary_json      JSON         NULL,
            error_message     TEXT         NULL,
            started_at        DATETIME(6)  NOT NULL DEFAULT (UTC_TIMESTAMP(6)),
            completed_at      DATETIME(6)  NULL,
            PRIMARY KEY (operation_id),
            INDEX idx_status      (status, started_at),
            INDEX idx_tenant_type (insight_tenant_id, operation_type, started_at),
            INDEX idx_author      (author_person_id, started_at)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
        """;

    // Mirror of Migrations/002_account_person_map.sql + 010_account_person_map_idx_by_account.sql.
    private const string AccountPersonMapDdl = """
        CREATE TABLE IF NOT EXISTS account_person_map (
            insight_tenant_id BINARY(16) NOT NULL,
            insight_source_type VARCHAR(100) NOT NULL,
            insight_source_id BINARY(16) NOT NULL,
            source_account_id VARCHAR(320) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
            person_id BINARY(16) NOT NULL,
            author_person_id BINARY(16) NOT NULL,
            reason VARCHAR(50) NOT NULL,
            valid_from TIMESTAMP(6) NOT NULL,
            valid_to TIMESTAMP(6) NULL,
            PRIMARY KEY (
                insight_tenant_id, insight_source_type, insight_source_id,
                source_account_id, valid_from
            ),
            INDEX idx_current (
                insight_tenant_id, insight_source_type, insight_source_id,
                source_account_id, valid_to
            ),
            INDEX idx_by_account (
                insight_tenant_id, source_account_id, valid_to
            ),
            INDEX idx_person_id (person_id),
            INDEX idx_tenant_person (insight_tenant_id, person_id),
            INDEX idx_valid_from (insight_tenant_id, valid_from)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
        """;

    private const string PersonRolesDdl = """
        CREATE TABLE IF NOT EXISTS person_roles (
            person_role_id    BINARY(16) NOT NULL,
            insight_tenant_id BINARY(16) NOT NULL,
            person_id         BINARY(16) NOT NULL,
            role_id           BINARY(16) NOT NULL,
            valid_from        DATETIME(6) NOT NULL,
            valid_to          DATETIME(6) NULL,
            author_person_id  BINARY(16) NOT NULL,
            reason            VARCHAR(500) NULL,
            created_at        DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6)),
            PRIMARY KEY (person_role_id),
            CONSTRAINT chk_person_roles_interval
                CHECK (valid_to IS NULL OR valid_from <= valid_to),
            INDEX idx_person_current (insight_tenant_id, person_id, role_id, valid_to),
            INDEX idx_role_current   (insight_tenant_id, role_id, valid_to)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
        """;

    // @admin_id bound at call-site from Domain.Services.Roles.Admin.
    private const string AdminRoleSeed = """
        INSERT INTO roles (role_id, name)
        VALUES (@admin_id, 'admin')
        ON DUPLICATE KEY UPDATE name = name
        """;

    // Mirror of Migrations/003_org_chart.sql. Kept inline here
    // so the fixture stays self-contained (no DbUp at test start).
    private const string OrgChartDdl = """
        CREATE TABLE IF NOT EXISTS org_chart (
            insight_tenant_id BINARY(16) NOT NULL,
            insight_source_type VARCHAR(30) NOT NULL,
            insight_source_id BINARY(16) NOT NULL,
            child_person_id BINARY(16) NOT NULL,
            parent_person_id BINARY(16) NULL,
            author_person_id BINARY(16) NOT NULL,
            reason VARCHAR(50) NULL,
            valid_from DATETIME(6) NOT NULL,
            valid_to DATETIME(6) NULL,
            PRIMARY KEY (
                insight_tenant_id, insight_source_type, insight_source_id,
                child_person_id, valid_from
            ),
            CONSTRAINT chk_no_self_loop CHECK (parent_person_id IS NULL OR child_person_id <> parent_person_id),
            INDEX idx_current_parent (
                insight_tenant_id, insight_source_type, insight_source_id,
                child_person_id, valid_to
            ),
            INDEX idx_current_children (
                insight_tenant_id, insight_source_type, insight_source_id,
                parent_person_id, valid_to
            ),
            INDEX idx_child_any_source  (insight_tenant_id, child_person_id, valid_to),
            INDEX idx_parent_any_source (insight_tenant_id, parent_person_id, valid_to),
            INDEX idx_valid_from (insight_tenant_id, valid_from)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
        """;
}

[CollectionDefinition(MariaDbCollection.Name)]
public sealed class MariaDbCollection : ICollectionFixture<MariaDbFixture>
{
    public const string Name = "MariaDB";
}
