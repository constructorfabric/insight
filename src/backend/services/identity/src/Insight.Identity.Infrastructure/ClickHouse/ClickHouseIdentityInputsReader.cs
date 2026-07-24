using System.Runtime.CompilerServices;
using Insight.Identity.Domain.Services;
using Octonica.ClickHouseClient;

namespace Insight.Identity.Infrastructure.ClickHouse;

/// <summary>
/// ClickHouse-backed <see cref="IIdentityInputsReader"/>. Streams rows
/// via the async DataReader so a large tenant does not buffer the full
/// result in memory — the seeder accumulates the working dictionaries
/// as rows arrive.
/// </summary>
public sealed class ClickHouseIdentityInputsReader : IIdentityInputsReader
{
    // The ORDER BY mirrors seed-persons-from-identity-input.py:
    // _synced_at DESC within (tenant, source_type, source_id,
    // source_account_id) so the FIRST row per account is the latest
    // observation — the seed algorithm uses this to pick an account's
    // current email and to detect a closed account (latest row is a
    // DELETE). DELETE rows are streamed too; they flag the account as
    // closed but never produce a persons observation.
    // The `_str` aliases avoid shadowing the source columns referenced
    // in WHERE (a same-name SELECT alias would otherwise be substituted
    // into the WHERE clause).
    //
    // HOTFIX (#1550) — TEMPORARY, identity-scoped, pre-release. The dbt
    // producer writes insight_tenant_id *hashed* — sipHash128 of whatever
    // raw string the connector was configured with (identity_inputs_from_
    // history.sql, documented there as a TEMPORARY cross-source join key) —
    // so the stored tenant never equals the caller's tenant and persons-seed
    // silently read 0 rows. There is no reliable representation to match
    // against (connector configs are free-form strings), so the tenant
    // filter is DROPPED for now: Insight deployments are single-tenant, all
    // identity_inputs rows belong to the deployment, and the seed writes
    // its output under the caller's tenant regardless of what the rows
    // carry (PersonsSeedService binds the request tenant, never the row's).
    //
    // MULTI-TENANT PREREQUISITE: the tenant filter MUST come back before any
    // multi-tenant deployment — without it every tenant's seed would read
    // (and re-file under itself) all other tenants' rows. Restoring it
    // requires the producer side to be fixed first: the tenant representation
    // unified end to end (dbt resolves real tenant UUIDs instead of hashing
    // free-form connector strings), then reinstate
    // `WHERE insight_tenant_id = {tenant:String}` here and in the Rust port.
    private const string StreamSql = """
        SELECT
            toString(insight_tenant_id) AS tenant_id_str,
            insight_source_type,
            toString(insight_source_id) AS source_id_str,
            source_account_id,
            value_type,
            value,
            _synced_at,
            operation_type
        FROM identity.identity_inputs
        WHERE operation_type    IN ('UPSERT', 'DELETE')
          AND value             IS NOT NULL
          AND value             != ''
        ORDER BY
            insight_source_type,
            insight_source_id,
            source_account_id,
            _synced_at DESC,
            value_type,
            value
        """;

    private readonly ClickHouseConnectionFactory _factory;

    public ClickHouseIdentityInputsReader(ClickHouseConnectionFactory factory)
    {
        _factory = factory;
    }

    public async IAsyncEnumerable<IdentityInputRow> StreamAsync(
        Guid tenantId,
        [EnumeratorCancellation] CancellationToken cancellationToken)
    {
        // tenantId is intentionally unused while the HOTFIX (#1550) drops the
        // tenant filter — kept so the interface (and the Rust port tracking
        // it) stays stable for when the filter comes back.
        _ = tenantId;
        await using var conn = await _factory.OpenAsync(cancellationToken).ConfigureAwait(false);
        await using var cmd = conn.CreateCommand(StreamSql);
        await using var reader = await cmd.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            yield return new IdentityInputRow(
                InsightTenantId:   Guid.Parse(reader.GetString(0)),
                InsightSourceType: reader.GetString(1),
                InsightSourceId:   Guid.Parse(reader.GetString(2)),
                SourceAccountId:   reader.GetString(3),
                ValueType:         reader.GetString(4),
                Value:             reader.GetString(5),
                SyncedAt:          reader.GetDateTime(6),
                IsDelete:          string.Equals(reader.GetString(7), "DELETE", StringComparison.Ordinal));
        }
    }
}
