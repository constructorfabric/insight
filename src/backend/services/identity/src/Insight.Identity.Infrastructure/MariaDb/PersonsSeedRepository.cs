using Insight.Identity.Domain.Services;
using MySqlConnector;

namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// MariaDB-backed <see cref="IPersonsSeedStore"/>. Reads feed the C#
/// resolver; <see cref="ApplyAsync"/> writes the resolved observations
/// and rebuilds both derived caches inside a single transaction so a
/// crash or cancellation can never leave the tenant's caches
/// cross-inconsistent.
/// </summary>
public sealed class PersonsSeedRepository : IPersonsSeedStore
{
    private readonly MariaDbConnectionFactory _factory;

    public PersonsSeedRepository(MariaDbConnectionFactory factory)
    {
        _factory = factory;
    }

    public async Task<IReadOnlyDictionary<SourceAccountKey, Guid>> GetKnownAccountBindingsAsync(
        Guid tenantId,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlPersonsSeed.KnownAccountBindings, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));

        var result = new Dictionary<SourceAccountKey, Guid>();
        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            var key = new SourceAccountKey(
                reader.GetString("insight_source_type"),
                new Guid((byte[])reader["insight_source_id"], bigEndian: true),
                reader.GetString("source_account_id"));
            result[key] = new Guid((byte[])reader["person_id"], bigEndian: true);
        }
        return result;
    }

    public async Task<IReadOnlyDictionary<string, Guid>> GetLatestEmailToPersonAsync(
        Guid tenantId,
        CancellationToken cancellationToken)
    {
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = new MySqlCommand(SqlPersonsSeed.LatestEmailToPerson, conn);
        cmd.Parameters.AddWithValue("@tenant_id", tenantId.ToByteArray(bigEndian: true));

        // Case-insensitive keys mirror the utf8mb4_unicode_ci collation
        // (ADR-0011): the SQL returns raw emails and the resolver looks
        // up by the source-cased email, so the dict must match case-
        // insensitively.
        var result = new Dictionary<string, Guid>(StringComparer.OrdinalIgnoreCase);
        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            result[reader.GetString("email")] = new Guid((byte[])reader["person_id"], bigEndian: true);
        }
        return result;
    }

    public async Task<SeedApplyResult> ApplyAsync(
        Guid tenantId,
        IReadOnlyList<PersonObservationRow> rows,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(rows);

        var tenantBin = tenantId.ToByteArray(bigEndian: true);

        // One connection + one transaction for the whole apply: observation
        // inserts, then both cache rebuilds. Either all of it commits or
        // none does — a crash or cancellation rolls back, so the tenant's
        // account_person_map / org_chart are never left cross-inconsistent.
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var tx = await conn.BeginTransactionAsync(cancellationToken).ConfigureAwait(false);

        var inserted = await InsertObservationsAsync(conn, tx, rows, cancellationToken).ConfigureAwait(false);
        await RebuildAccountPersonMapAsync(conn, tx, tenantBin, cancellationToken).ConfigureAwait(false);
        var edges = await RebuildOrgChartAsync(conn, tx, tenantBin, cancellationToken).ConfigureAwait(false);

        await tx.CommitAsync(cancellationToken).ConfigureAwait(false);
        return new SeedApplyResult(inserted, edges);
    }

    private static async Task<int> InsertObservationsAsync(
        MySqlConnection conn, MySqlTransaction tx,
        IReadOnlyList<PersonObservationRow> rows, CancellationToken cancellationToken)
    {
        if (rows.Count == 0)
        {
            return 0;
        }

        // One reused command. Per-row execute on a single session is fast
        // enough for a background operation; multi-row VALUES batching is a
        // future optimisation if seed wallclock becomes a concern.
        await using var cmd = new MySqlCommand(SqlPersonsSeed.InsertObservation, conn, tx);
        var pValueType = cmd.Parameters.Add("@value_type", MySqlDbType.VarChar);
        var pSourceType = cmd.Parameters.Add("@source_type", MySqlDbType.VarChar);
        var pSourceId = cmd.Parameters.Add("@source_id", MySqlDbType.Binary);
        var pTenantId = cmd.Parameters.Add("@tenant_id", MySqlDbType.Binary);
        var pValueId = cmd.Parameters.Add("@value_id", MySqlDbType.VarChar);
        var pValueFullText = cmd.Parameters.Add("@value_full_text", MySqlDbType.VarChar);
        var pValue = cmd.Parameters.Add("@value", MySqlDbType.Text);
        var pPersonId = cmd.Parameters.Add("@person_id", MySqlDbType.Binary);
        var pAuthor = cmd.Parameters.Add("@author_person_id", MySqlDbType.Binary);
        var pReason = cmd.Parameters.Add("@reason", MySqlDbType.VarChar);
        var pCreatedAt = cmd.Parameters.Add("@created_at", MySqlDbType.DateTime);
        await cmd.PrepareAsync(cancellationToken).ConfigureAwait(false);

        // INSERT IGNORE returns 1 for a freshly-written row and 0 for a
        // duplicate suppressed by the unique key, so this sum is the
        // NET-NEW count — a pure re-seed yields 0.
        var inserted = 0;
        foreach (var row in rows)
        {
            cancellationToken.ThrowIfCancellationRequested();
            pValueType.Value = row.ValueType;
            pSourceType.Value = row.InsightSourceType;
            pSourceId.Value = row.InsightSourceId.ToByteArray(bigEndian: true);
            pTenantId.Value = row.InsightTenantId.ToByteArray(bigEndian: true);
            pValueId.Value = (object?)row.ValueId ?? DBNull.Value;
            pValueFullText.Value = (object?)row.ValueFullText ?? DBNull.Value;
            pValue.Value = (object?)row.Value ?? DBNull.Value;
            pPersonId.Value = row.PersonId.ToByteArray(bigEndian: true);
            pAuthor.Value = row.AuthorPersonId.ToByteArray(bigEndian: true);
            pReason.Value = row.Reason;
            pCreatedAt.Value = row.CreatedAt;
            inserted += await cmd.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        }
        return inserted;
    }

    private static async Task RebuildAccountPersonMapAsync(
        MySqlConnection conn, MySqlTransaction tx, byte[] tenantBin, CancellationToken cancellationToken)
    {
        await using (var del = new MySqlCommand(SqlPersonsSeed.DeleteAccountPersonMapForTenant, conn, tx))
        {
            del.Parameters.AddWithValue("@tenant_id", tenantBin);
            await del.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        }
        await using (var ins = new MySqlCommand(SqlPersonsSeed.InsertAccountPersonMapForTenant, conn, tx))
        {
            ins.Parameters.AddWithValue("@tenant_id", tenantBin);
            await ins.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        }
    }

    private static async Task<int> RebuildOrgChartAsync(
        MySqlConnection conn, MySqlTransaction tx, byte[] tenantBin, CancellationToken cancellationToken)
    {
        await using (var del = new MySqlCommand(SqlPersonsSeed.DeleteOrgChartForTenant, conn, tx))
        {
            del.Parameters.AddWithValue("@tenant_id", tenantBin);
            await del.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        }
        await using (var ins = new MySqlCommand(SqlPersonsSeed.InsertOrgChartForTenant, conn, tx))
        {
            ins.Parameters.AddWithValue("@tenant_id", tenantBin);
            return await ins.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        }
    }
}
