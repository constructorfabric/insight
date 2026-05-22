using Insight.Identity.Domain.Services;
using MySqlConnector;

namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// MariaDB-backed <see cref="IRolesReader"/> + <see cref="IPersonRolesReader"/>.
/// Both ports share one repository because the SQL surface is tiny and
/// the two tables are joined-at-the-hip in every realistic call path
/// (resolve the role row → check the assignment).
/// </summary>
public sealed class RolesRepository : IRolesReader, IPersonRolesReader
{
    private readonly MariaDbConnectionFactory _factory;

    public RolesRepository(MariaDbConnectionFactory factory)
    {
        _factory = factory;
    }

    public async Task<Role?> GetByNameAsync(string name, CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(name);
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.RoleByName, conn);
        cmd.Parameters.AddWithValue("@name", name);
        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        if (!await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            return null;
        }
        return new Role(
            RoleId: new Guid((byte[])reader["role_id"], bigEndian: true),
            Name: reader.GetString("name"));
    }

    public async Task<IReadOnlyList<Role>> ListAllAsync(CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.ListAllRoles, conn);
        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        var list = new List<Role>();
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            list.Add(new Role(
                RoleId: new Guid((byte[])reader["role_id"], bigEndian: true),
                Name: reader.GetString("name")));
        }
        return list;
    }

    public async Task<bool> HasActiveRoleAsync(
        Guid tenantId,
        Guid personId,
        Guid roleId,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.HasActivePersonRole, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@person_id", personId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@role_id", roleId.ToByteArray(bigEndian: true));
        var raw = await cmd.ExecuteScalarAsync(cancellationToken).ConfigureAwait(false);
        return Convert.ToBoolean(raw, System.Globalization.CultureInfo.InvariantCulture);
    }

    public async Task<IReadOnlyList<PersonRole>> GetActiveByPersonAsync(
        Guid tenantId,
        Guid personId,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.ActivePersonRolesByPerson, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@person_id", personId.ToByteArray(bigEndian: true));
        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        var list = new List<PersonRole>();
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            var idxReason = reader.GetOrdinal("reason");
            list.Add(new PersonRole(
                PersonRoleId:    new Guid((byte[])reader["person_role_id"], bigEndian: true),
                InsightTenantId: new Guid((byte[])reader["insight_tenant_id"], bigEndian: true),
                PersonId:        new Guid((byte[])reader["person_id"], bigEndian: true),
                RoleId:          new Guid((byte[])reader["role_id"], bigEndian: true),
                ValidFrom:       reader.GetDateTime("valid_from"),
                ValidTo:         reader.IsDBNull(reader.GetOrdinal("valid_to"))
                                     ? null
                                     : reader.GetDateTime("valid_to"),
                AuthorPersonId:  new Guid((byte[])reader["author_person_id"], bigEndian: true),
                Reason:          reader.IsDBNull(idxReason) ? null : reader.GetString(idxReason),
                CreatedAt:       reader.GetDateTime("created_at")));
        }
        return list;
    }
}
