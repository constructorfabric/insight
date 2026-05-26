using System.Globalization;
using System.Text;
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

    // ── IRolesReader ────────────────────────────────────────────────

    public async Task<Role?> GetByNameAsync(string name, CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(name);
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.RoleByName, conn);
        cmd.Parameters.AddWithValue("@name", name);
        return await ReadOneRoleAsync(cmd, cancellationToken).ConfigureAwait(false);
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

    public async Task<Role?> GetRoleByIdAsync(Guid roleId, CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.RoleById, conn);
        cmd.Parameters.AddWithValue("@role_id", roleId.ToByteArray(bigEndian: true));
        return await ReadOneRoleAsync(cmd, cancellationToken).ConfigureAwait(false);
    }

    public async Task<Guid> InsertRoleAsync(string name, CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(name);
        var roleId = Guid.NewGuid();
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.InsertRole, conn);
        cmd.Parameters.AddWithValue("@role_id", roleId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@name", name);
        await cmd.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        return roleId;
    }

    /// <summary>
    /// Atomic hard-delete of a role: refuses the write when any active
    /// <c>person_roles</c> row references the role (orphan guard, see
    /// ADR-0013). Returns <c>1</c> on success, <c>0</c> when either the
    /// role is gone or the guard fired — the caller disambiguates with
    /// a second read.
    /// </summary>
    public async Task<int> TryDeleteRoleIfUnusedAsync(Guid roleId, CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.TryDeleteRoleIfUnused, conn);
        cmd.Parameters.AddWithValue("@role_id", roleId.ToByteArray(bigEndian: true));
        return await cmd.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
    }

    public async Task<int> CountActiveAssignmentsByRoleAnyTenantAsync(Guid roleId, CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.CountActivePersonRolesByRoleAnyTenant, conn);
        cmd.Parameters.AddWithValue("@role_id", roleId.ToByteArray(bigEndian: true));
        var raw = await cmd.ExecuteScalarAsync(cancellationToken).ConfigureAwait(false);
        return Convert.ToInt32(raw, CultureInfo.InvariantCulture);
    }

    // ── IPersonRolesReader ──────────────────────────────────────────

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
        cmd.Parameters.AddWithValue("@role_id",   roleId.ToByteArray(bigEndian: true));
        var raw = await cmd.ExecuteScalarAsync(cancellationToken).ConfigureAwait(false);
        return Convert.ToBoolean(raw, CultureInfo.InvariantCulture);
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
        return await ReadPersonRolesAsync(cmd, cancellationToken).ConfigureAwait(false);
    }

    public async Task<PersonRole?> GetPersonRoleByIdAsync(Guid tenantId, Guid personRoleId, CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.PersonRoleById, conn);
        cmd.Parameters.AddWithValue("@tenant_id",      tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@person_role_id", personRoleId.ToByteArray(bigEndian: true));
        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        if (!await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            return null;
        }
        return ReadPersonRole(reader);
    }

    public async Task<PagedResult<PersonRole>> ListAsync(
        Guid tenantId,
        Guid? filterByPerson,
        Guid? filterByRole,
        bool activeOnly,
        PageRequest page,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(page);
        var clamped = page.WithClampedLimit();

        var sb = new StringBuilder(SqlRoles.PersonRoleListBase);
        if (filterByPerson is not null) sb.Append(" AND person_id = @person_id");
        if (filterByRole is not null)   sb.Append(" AND role_id   = @role_id");
        if (activeOnly)                 sb.Append(" AND valid_to IS NULL");
        sb.Append(" ORDER BY created_at DESC, person_role_id DESC LIMIT @limit");

        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(sb.ToString(), conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        if (filterByPerson is { } p) cmd.Parameters.AddWithValue("@person_id", p.ToByteArray(bigEndian: true));
        if (filterByRole   is { } r) cmd.Parameters.AddWithValue("@role_id",   r.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@limit", clamped.Limit);
        var list = await ReadPersonRolesAsync(cmd, cancellationToken).ConfigureAwait(false);
        return new PagedResult<PersonRole>(list, NextCursor: null);
    }

    public async Task<int> CountActiveByRoleAsync(Guid tenantId, Guid roleId, CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.CountActivePersonRolesByRole, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@role_id",   roleId.ToByteArray(bigEndian: true));
        var raw = await cmd.ExecuteScalarAsync(cancellationToken).ConfigureAwait(false);
        return Convert.ToInt32(raw, CultureInfo.InvariantCulture);
    }

    public async Task<Guid> InsertPersonRoleAsync(
        Guid tenantId,
        Guid personId,
        Guid roleId,
        DateTime? validFrom,
        Guid authorPersonId,
        string? reason,
        CancellationToken cancellationToken)
    {
        var personRoleId = Guid.NewGuid();
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlRoles.InsertPersonRole, conn);
        cmd.Parameters.AddWithValue("@person_role_id", personRoleId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant_id",      tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@person_id",      personId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@role_id",        roleId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@valid_from",     validFrom is { } vf ? vf : (object)DBNull.Value);
        cmd.Parameters.AddWithValue("@author_person_id", authorPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@reason",         reason is null ? (object)DBNull.Value : reason);
        await cmd.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        return personRoleId;
    }

    /// <summary>
    /// Atomic soft-delete of a <c>person_roles</c> row with last-admin
    /// protection (see ADR-0014). Single UPDATE: the guard and the
    /// write happen in the same statement, so there is no read-check-
    /// write window for two concurrent admin revokes to slip through.
    /// Returns <c>1</c> on success, <c>0</c> when the row is already
    /// revoked / missing OR the last-admin guard fired — the caller
    /// disambiguates with a second read.
    /// </summary>
    public async Task<int> TrySoftDeletePersonRoleProtectingLastAdminAsync(
        Guid personRoleId,
        Guid adminRoleId,
        string? reason,
        CancellationToken cancellationToken)
    {
        // Transactional lock-then-update: the derived-table COUNT inside
        // the UPDATE alone is implementation-dependent (snapshot read in
        // some MariaDB versions), so two concurrent revokes targeting
        // different admin rows in a 2-admin tenant could both observe
        // count>1 and both commit, leaving zero admins. We close the
        // race by acquiring row-level write locks on every active admin
        // in the target's tenant first; the UPDATE that follows then
        // sees a serialised view of the count.
        var personRoleBytes = personRoleId.ToByteArray(bigEndian: true);
        var adminRoleBytes  = adminRoleId.ToByteArray(bigEndian: true);

        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var tx = await conn
            .BeginTransactionAsync(System.Data.IsolationLevel.RepeatableRead, cancellationToken)
            .ConfigureAwait(false);

        await using (var lockCmd = new MySqlCommand(SqlRoles.LockActiveAdminsInTenantForUpdate, conn, tx))
        {
            lockCmd.Parameters.AddWithValue("@person_role_id", personRoleBytes);
            lockCmd.Parameters.AddWithValue("@admin_role_id",  adminRoleBytes);
            // ExecuteScalarAsync drains the single COUNT row and forces
            // MariaDB to finish the scan — every active admin row in the
            // tenant is locked by the time we move on.
            await lockCmd.ExecuteScalarAsync(cancellationToken).ConfigureAwait(false);
        }

        await using var cmd = new MySqlCommand(SqlRoles.TrySoftDeletePersonRoleProtectingLastAdmin, conn, tx);
        cmd.Parameters.AddWithValue("@person_role_id", personRoleBytes);
        cmd.Parameters.AddWithValue("@admin_role_id",  adminRoleBytes);
        cmd.Parameters.AddWithValue("@reason",         reason is null ? (object)DBNull.Value : reason);
        var rows = await cmd.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);

        await tx.CommitAsync(cancellationToken).ConfigureAwait(false);
        return rows;
    }

    // ── Read helpers ────────────────────────────────────────────────

    private static async Task<Role?> ReadOneRoleAsync(MySqlCommand cmd, CancellationToken ct)
    {
        await using var reader = await cmd.ExecuteReaderAsync(ct).ConfigureAwait(false);
        if (!await reader.ReadAsync(ct).ConfigureAwait(false))
        {
            return null;
        }
        return new Role(
            RoleId: new Guid((byte[])reader["role_id"], bigEndian: true),
            Name: reader.GetString("name"));
    }

    private static async Task<List<PersonRole>> ReadPersonRolesAsync(MySqlCommand cmd, CancellationToken ct)
    {
        await using var reader = await cmd.ExecuteReaderAsync(ct).ConfigureAwait(false);
        var list = new List<PersonRole>();
        while (await reader.ReadAsync(ct).ConfigureAwait(false))
        {
            list.Add(ReadPersonRole(reader));
        }
        return list;
    }

    private static PersonRole ReadPersonRole(MySqlDataReader reader)
    {
        var idxValidTo = reader.GetOrdinal("valid_to");
        var idxReason = reader.GetOrdinal("reason");
        return new PersonRole(
            PersonRoleId:    new Guid((byte[])reader["person_role_id"], bigEndian: true),
            InsightTenantId: new Guid((byte[])reader["insight_tenant_id"], bigEndian: true),
            PersonId:        new Guid((byte[])reader["person_id"], bigEndian: true),
            RoleId:          new Guid((byte[])reader["role_id"], bigEndian: true),
            ValidFrom:       reader.GetDateTime("valid_from"),
            ValidTo:         reader.IsDBNull(idxValidTo) ? null : reader.GetDateTime("valid_to"),
            AuthorPersonId:  new Guid((byte[])reader["author_person_id"], bigEndian: true),
            Reason:          reader.IsDBNull(idxReason) ? null : reader.GetString(idxReason),
            CreatedAt:       reader.GetDateTime("created_at"));
    }
}
