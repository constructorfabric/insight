using Insight.Identity.Domain.Services;
using MySqlConnector;

namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// MariaDB-backed <see cref="ISubchartReader"/>. One SQL round-trip per
/// call — the recursive CTE plus the latest-observation pass live in a
/// single statement (<see cref="SqlSubchart.GetSubchart"/>).
/// </summary>
public sealed class SubchartRepository : ISubchartReader
{
    private readonly MariaDbConnectionFactory _factory;

    public SubchartRepository(MariaDbConnectionFactory factory)
    {
        _factory = factory;
    }

    public async Task<IReadOnlyList<SubchartFlatNode>> GetSubchartAsync(
        Guid tenantId,
        Guid rootPersonId,
        string orgChartSourceType,
        int? maxDepth,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlSubchart.GetSubchart, conn);
        cmd.Parameters.AddWithValue("@tenant_id",      tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@root_person_id", rootPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@source_type",    orgChartSourceType);
        cmd.Parameters.AddWithValue("@max_depth",      (object?)maxDepth ?? DBNull.Value);
        return await ReadFlatAsync(cmd, cancellationToken).ConfigureAwait(false);
    }

    public async Task<IReadOnlyList<SubchartFlatNode>> GetForestAsync(
        Guid tenantId,
        Guid viewerPersonId,
        string orgChartSourceType,
        int? maxDepth,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlSubchart.GetForest, conn);
        cmd.Parameters.AddWithValue("@tenant_id",        tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@viewer_person_id", viewerPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@source_type",      orgChartSourceType);
        cmd.Parameters.AddWithValue("@max_depth",        (object?)maxDepth ?? DBNull.Value);
        return await ReadFlatAsync(cmd, cancellationToken).ConfigureAwait(false);
    }

    private static async Task<IReadOnlyList<SubchartFlatNode>> ReadFlatAsync(
        MySqlCommand cmd, CancellationToken cancellationToken)
    {
        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        var list = new List<SubchartFlatNode>();
        var idxParent  = reader.GetOrdinal("parent_person_id");
        var idxEmail   = reader.GetOrdinal("email");
        var idxDisplay = reader.GetOrdinal("display_name");
        var idxJob     = reader.GetOrdinal("job_title");
        var idxStatus  = reader.GetOrdinal("status");
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            list.Add(new SubchartFlatNode(
                PersonId:       new Guid((byte[])reader["person_id"], bigEndian: true),
                ParentPersonId: reader.IsDBNull(idxParent)
                    ? null
                    : new Guid((byte[])reader["parent_person_id"], bigEndian: true),
                Depth:          reader.GetInt32("depth"),
                Email:          reader.IsDBNull(idxEmail)   ? null : reader.GetString("email"),
                DisplayName:    reader.IsDBNull(idxDisplay) ? null : reader.GetString("display_name"),
                JobTitle:       reader.IsDBNull(idxJob)     ? null : reader.GetString("job_title"),
                Status:         reader.IsDBNull(idxStatus)  ? null : reader.GetString("status")));
        }
        return list;
    }
}
