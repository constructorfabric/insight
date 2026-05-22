using Insight.Identity.Domain.Services;
using MySqlConnector;

namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// MariaDB-backed <see cref="IVisibilityReader"/>. Lists a viewer's
/// active grants; the recursive `can_see` predicate is composed on top
/// of this reader by <c>VisibilityService</c>.
/// </summary>
public sealed class VisibilityRepository : IVisibilityReader
{
    private readonly MariaDbConnectionFactory _factory;

    public VisibilityRepository(MariaDbConnectionFactory factory)
    {
        _factory = factory;
    }

    public async Task<IReadOnlyList<Visibility>> GetActiveGrantsByViewerAsync(
        Guid tenantId,
        Guid viewerPersonId,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlVisibility.ActiveGrantsByViewer, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@viewer_person_id", viewerPersonId.ToByteArray(bigEndian: true));

        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        var list = new List<Visibility>();
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            var idxViewed = reader.GetOrdinal("viewed_person_id");
            var idxReason = reader.GetOrdinal("reason");
            list.Add(new Visibility(
                VisibilityId:     new Guid((byte[])reader["visibility_id"], bigEndian: true),
                InsightTenantId:  new Guid((byte[])reader["insight_tenant_id"], bigEndian: true),
                ViewerPersonId:   new Guid((byte[])reader["viewer_person_id"], bigEndian: true),
                ViewedPersonId:   reader.IsDBNull(idxViewed)
                                      ? null
                                      : new Guid((byte[])reader["viewed_person_id"], bigEndian: true),
                ValidFrom:        reader.GetDateTime("valid_from"),
                ValidTo:          reader.IsDBNull(reader.GetOrdinal("valid_to"))
                                      ? null
                                      : reader.GetDateTime("valid_to"),
                AuthorPersonId:   new Guid((byte[])reader["author_person_id"], bigEndian: true),
                Reason:           reader.IsDBNull(idxReason) ? null : reader.GetString(idxReason),
                CreatedAt:        reader.GetDateTime("created_at")));
        }
        return list;
    }
}
