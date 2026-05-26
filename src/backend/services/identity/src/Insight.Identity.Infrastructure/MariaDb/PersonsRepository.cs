using Insight.Identity.Domain;
using Insight.Identity.Domain.Services;
using MySqlConnector;

namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// MariaDB-backed <see cref="IPersonsReader"/>. UUIDs round-trip as
/// raw 16-byte big-endian values (RFC 4122 wire order) — see
/// <c>cpt-insightspec-nfr-identity-uuid-roundtrip</c>.
/// </summary>
public sealed class PersonsRepository : IPersonsReader
{
    private readonly MariaDbConnectionFactory _factory;

    public PersonsRepository(MariaDbConnectionFactory factory)
    {
        _factory = factory;
    }

    public async Task<Guid?> ResolvePersonIdByEmailAsync(
        Guid tenantId,
        string email,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(Sql.ResolvePersonIdByEmail, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@email", email);
        var raw = await cmd.ExecuteScalarAsync(cancellationToken).ConfigureAwait(false);
        return raw is byte[] bytes && bytes.Length == 16 ? new Guid(bytes, bigEndian: true) : null;
    }

    public async Task<IReadOnlyList<PersonObservation>> GetLatestObservationsAsync(
        Guid tenantId,
        Guid personId,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(Sql.LatestObservationsForPerson, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@person_id", personId.ToByteArray(bigEndian: true));

        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        var list = new List<PersonObservation>();
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            var personBytes = (byte[])reader["person_id"];
            var sourceIdBytes = (byte[])reader["insight_source_id"];
            list.Add(new PersonObservation(
                PersonId: new Guid(personBytes, bigEndian: true),
                InsightSourceType: reader.GetString("insight_source_type"),
                InsightSourceId: new Guid(sourceIdBytes, bigEndian: true),
                ValueType: reader.GetString("value_type"),
                ValueEffective: reader.GetString("value_effective"),
                CreatedAt: reader.GetDateTime("created_at")));
        }
        return list;
    }

    public Task<IReadOnlyList<OrgChartEdge>> GetCurrentParentsAsync(
        Guid tenantId,
        Guid childPersonId,
        CancellationToken cancellationToken)
        => ReadEdgesAsync(
            SqlOrgChart.CurrentParentsForChild,
            tenantId,
            ("@child_person_id", childPersonId),
            cancellationToken);

    public Task<IReadOnlyList<OrgChartEdge>> GetCurrentChildrenAsync(
        Guid tenantId,
        Guid parentPersonId,
        CancellationToken cancellationToken)
        => ReadEdgesAsync(
            SqlOrgChart.CurrentChildrenForParent,
            tenantId,
            ("@parent_person_id", parentPersonId),
            cancellationToken);

    private async Task<IReadOnlyList<OrgChartEdge>> ReadEdgesAsync(
        string sql,
        Guid tenantId,
        (string name, Guid value) bound,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue(bound.name, bound.value.ToByteArray(bigEndian: true));

        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        var edges = new List<OrgChartEdge>();
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            var childBytes  = (byte[])reader["child_person_id"];
            var parentBytes = (byte[])reader["parent_person_id"];
            var sourceBytes = (byte[])reader["insight_source_id"];
            edges.Add(new OrgChartEdge(
                InsightSourceType: reader.GetString("insight_source_type"),
                InsightSourceId:   new Guid(sourceBytes, bigEndian: true),
                ChildPersonId:     new Guid(childBytes,  bigEndian: true),
                ParentPersonId:    new Guid(parentBytes, bigEndian: true),
                ValidFrom:         reader.GetDateTime("valid_from")));
        }
        return edges;
    }

    public async Task<bool> PingAsync(CancellationToken cancellationToken)
    {
        try
        {
            await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
            await using var cmd = new MySqlCommand(Sql.Healthcheck, conn);
            var raw = await cmd.ExecuteScalarAsync(cancellationToken).ConfigureAwait(false);
            return raw is not null;
        }
        catch (MySqlException)
        {
            return false;
        }
    }

    public async Task<IReadOnlyList<Guid>> ResolvePersonIdsByEmailAsync(
        Guid tenantId,
        string email,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlProfiles.ResolvePersonIdsByEmail, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@value", email);
        return await ReadPersonIdsAsync(cmd, cancellationToken).ConfigureAwait(false);
    }

    public async Task<IReadOnlyList<Guid>> ResolvePersonIdsBySourceIdAsync(
        Guid tenantId,
        string sourceType,
        Guid sourceId,
        string value,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlProfiles.ResolvePersonIdsBySourceId, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@source_type", sourceType);
        cmd.Parameters.AddWithValue("@source_id", sourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@value", value);
        return await ReadPersonIdsAsync(cmd, cancellationToken).ConfigureAwait(false);
    }

    public async Task<Guid?> ResolvePersonIdByAccountIdAsync(
        Guid tenantId,
        string accountId,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlAuth.ResolvePersonIdByAccountId, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@account_id", accountId);
        var raw = await cmd.ExecuteScalarAsync(cancellationToken).ConfigureAwait(false);
        return raw is byte[] bytes && bytes.Length == 16 ? new Guid(bytes, bigEndian: true) : null;
    }

    public async Task<IReadOnlyList<PersonSourceId>> GetCurrentSourceIdsAsync(
        Guid tenantId,
        Guid personId,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlProfiles.CurrentSourceIdsForPerson, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@person_id", personId.ToByteArray(bigEndian: true));

        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        var list = new List<PersonSourceId>();
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            var sourceIdBytes = (byte[])reader["insight_source_id"];
            list.Add(new PersonSourceId(
                InsightSourceType: reader.GetString("insight_source_type"),
                InsightSourceId: new Guid(sourceIdBytes, bigEndian: true),
                Value: reader.GetString("value")));
        }
        return list;
    }

    private static async Task<IReadOnlyList<Guid>> ReadPersonIdsAsync(
        MySqlCommand cmd,
        CancellationToken cancellationToken)
    {
        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        var ids = new List<Guid>();
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            var bytes = (byte[])reader["person_id"];
            ids.Add(new Guid(bytes, bigEndian: true));
        }
        return ids;
    }
}
