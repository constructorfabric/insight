using Insight.Identity.Domain.Services;
using Microsoft.Extensions.Logging;
using MySqlConnector;

namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// One-shot startup hook: inserts the bootstrap admin into
/// <c>person_roles</c> if it's not already there. Chicken-and-egg
/// solution — CRUD endpoints require an admin to make further grants,
/// so a fresh tenant needs the very first admin to come from somewhere
/// outside the API.
/// </summary>
public sealed class BootstrapAdminRunner
{
    private static readonly Action<ILogger, Guid, Guid, Exception?> LogInserted =
        LoggerMessage.Define<Guid, Guid>(
            LogLevel.Information,
            new EventId(1, nameof(LogInserted)),
            "Bootstrap admin seeded: tenant={Tenant} person={Person}");

    private static readonly Action<ILogger, Guid, Guid, Exception?> LogAlreadyPresent =
        LoggerMessage.Define<Guid, Guid>(
            LogLevel.Information,
            new EventId(2, nameof(LogAlreadyPresent)),
            "Bootstrap admin already present: tenant={Tenant} person={Person}");

    private static readonly Action<ILogger, Exception?> LogSkippedNoTenant =
        LoggerMessage.Define(
            LogLevel.Warning,
            new EventId(3, nameof(LogSkippedNoTenant)),
            "Bootstrap admin requested but tenant_default_id is not set — skipping");

    public static async Task RunAsync(
        MariaDbConnectionFactory factory,
        Guid? tenantId,
        Guid? bootstrapAdminPersonId,
        ILogger logger,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(factory);
        ArgumentNullException.ThrowIfNull(logger);

        if (bootstrapAdminPersonId is not { } person)
        {
            return;
        }
        if (tenantId is not { } tenant)
        {
            LogSkippedNoTenant(logger, null);
            return;
        }

        const string sql = """
            INSERT INTO person_roles
                (person_role_id, insight_tenant_id, person_id, role_id,
                 valid_from, valid_to, author_person_id, reason)
            SELECT
                @new_person_role_id, @tenant, @person, @role,
                UTC_TIMESTAMP(6), NULL, @person, 'bootstrap'
            WHERE NOT EXISTS (
                SELECT 1 FROM person_roles
                WHERE insight_tenant_id = @tenant
                  AND person_id         = @person
                  AND role_id           = @role
                  AND valid_to IS NULL
            )
            """;
        await using var conn = await factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@new_person_role_id", Guid.NewGuid().ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant", tenant.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@person", person.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@role",   Roles.Admin.ToByteArray(bigEndian: true));
        var rows = await cmd.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        if (rows > 0)
        {
            LogInserted(logger, tenant, person, null);
        }
        else
        {
            LogAlreadyPresent(logger, tenant, person, null);
        }
    }
}
