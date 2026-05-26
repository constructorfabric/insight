using System.Globalization;
using System.Text;
using Insight.Identity.Domain.Services;
using MySqlConnector;

namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// MariaDB-backed <see cref="IVisibilityReader"/> plus the write
/// side (INSERT, soft-delete) consumed by the CRUD endpoints.
/// </summary>
public sealed class VisibilityRepository : IVisibilityReader
{
    private readonly MariaDbConnectionFactory _factory;

    public VisibilityRepository(MariaDbConnectionFactory factory)
    {
        _factory = factory;
    }

    public async Task<IReadOnlyList<Visibility>> GetActiveVisibilityGrantsByViewerAsync(
        Guid tenantId,
        Guid viewerPersonId,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlVisibility.ActiveGrantsByViewer, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@viewer_person_id", viewerPersonId.ToByteArray(bigEndian: true));
        return await ReadListAsync(cmd, cancellationToken).ConfigureAwait(false);
    }

    public async Task<bool> IsTargetInVisibleSetAsync(
        Guid tenantId,
        Guid viewerPersonId,
        Guid targetPersonId,
        string orgChartSourceType,
        CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrEmpty(orgChartSourceType);
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlVisibility.IsTargetInVisibleSet, conn);
        cmd.Parameters.AddWithValue("@tenant_id",        tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@viewer_person_id", viewerPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@target_person_id", targetPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@org_source_type",  orgChartSourceType);
        var raw = await cmd.ExecuteScalarAsync(cancellationToken).ConfigureAwait(false);
        return Convert.ToBoolean(raw, CultureInfo.InvariantCulture);
    }

    public async Task<Visibility?> GetByIdAsync(Guid tenantId, Guid visibilityId, CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlVisibility.GetById, conn);
        cmd.Parameters.AddWithValue("@tenant_id",     tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@visibility_id", visibilityId.ToByteArray(bigEndian: true));
        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        if (!await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            return null;
        }
        return Read(reader);
    }

    public async Task<PagedResult<Visibility>> ListAsync(
        Guid tenantId,
        Guid? filterByViewer,
        Guid? filterByViewed,
        bool activeOnly,
        PageRequest page,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(page);
        var clamped = page.WithClampedLimit();

        var sb = new StringBuilder(SqlVisibility.ListBase);
        if (filterByViewer is not null) sb.Append(" AND viewer_person_id = @viewer_person_id");
        if (filterByViewed is not null) sb.Append(" AND viewed_person_id = @viewed_person_id");
        if (activeOnly)                 sb.Append(" AND valid_to IS NULL");
        sb.Append(" ORDER BY created_at DESC, visibility_id DESC LIMIT @limit");

        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(sb.ToString(), conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        if (filterByViewer is { } v)   cmd.Parameters.AddWithValue("@viewer_person_id", v.ToByteArray(bigEndian: true));
        if (filterByViewed is { } vd)  cmd.Parameters.AddWithValue("@viewed_person_id", vd.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@limit", clamped.Limit);
        var list = await ReadListAsync(cmd, cancellationToken).ConfigureAwait(false);
        // Cursor pagination is a step-4-bookmark — for now the next-cursor is always null.
        return new PagedResult<Visibility>(list, NextCursor: null);
    }

    public async Task<Guid> InsertAsync(
        Guid tenantId,
        Guid viewerPersonId,
        Guid? viewedPersonId,
        DateTime? validFrom,
        Guid authorPersonId,
        string? reason,
        CancellationToken cancellationToken)
    {
        var visibilityId = Guid.NewGuid();
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlVisibility.Insert, conn);
        cmd.Parameters.AddWithValue("@visibility_id",    visibilityId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant_id",        tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@viewer_person_id", viewerPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@viewed_person_id", viewedPersonId is { } v ? v.ToByteArray(bigEndian: true) : (object)DBNull.Value);
        cmd.Parameters.AddWithValue("@valid_from",       validFrom is { } vf ? vf : (object)DBNull.Value);
        cmd.Parameters.AddWithValue("@author_person_id", authorPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@reason",           reason is null ? (object)DBNull.Value : reason);
        await cmd.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        return visibilityId;
    }

    public async Task<int> SoftDeleteAsync(Guid tenantId, Guid visibilityId, string? reason, CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlVisibility.SoftDelete, conn);
        cmd.Parameters.AddWithValue("@tenant_id",     tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@visibility_id", visibilityId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@reason",        reason is null ? (object)DBNull.Value : reason);
        return await cmd.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
    }

    private static async Task<List<Visibility>> ReadListAsync(MySqlCommand cmd, CancellationToken ct)
    {
        await using var reader = await cmd.ExecuteReaderAsync(ct).ConfigureAwait(false);
        var list = new List<Visibility>();
        while (await reader.ReadAsync(ct).ConfigureAwait(false))
        {
            list.Add(Read(reader));
        }
        return list;
    }

    private static Visibility Read(MySqlDataReader reader)
    {
        var idxViewed = reader.GetOrdinal("viewed_person_id");
        var idxValidTo = reader.GetOrdinal("valid_to");
        var idxReason = reader.GetOrdinal("reason");
        return new Visibility(
            VisibilityId:    new Guid((byte[])reader["visibility_id"], bigEndian: true),
            InsightTenantId: new Guid((byte[])reader["insight_tenant_id"], bigEndian: true),
            ViewerPersonId:  new Guid((byte[])reader["viewer_person_id"], bigEndian: true),
            ViewedPersonId:  reader.IsDBNull(idxViewed)
                                 ? null
                                 : new Guid((byte[])reader["viewed_person_id"], bigEndian: true),
            ValidFrom:       reader.GetDateTime("valid_from"),
            ValidTo:         reader.IsDBNull(idxValidTo) ? null : reader.GetDateTime("valid_to"),
            AuthorPersonId:  new Guid((byte[])reader["author_person_id"], bigEndian: true),
            Reason:          reader.IsDBNull(idxReason) ? null : reader.GetString(idxReason),
            CreatedAt:       reader.GetDateTime("created_at"));
    }
}
