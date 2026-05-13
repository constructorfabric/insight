namespace Insight.Identity.Domain.Services;

/// <summary>
/// Repository abstraction the lookup service depends on. The infrastructure
/// project supplies a MariaDB-backed implementation; tests can stub the
/// interface directly.
/// </summary>
public interface IPersonsReader
{
    /// <summary>
    /// Resolve a single <c>person_id</c> from a lookup email. Returns
    /// <c>null</c> when no current observation in the tenant has
    /// <c>value_type='email'</c> = <paramref name="emailLowercase"/>.
    /// </summary>
    Task<Guid?> ResolvePersonIdByEmailAsync(
        Guid tenantId,
        string emailLowercase,
        CancellationToken cancellationToken);

    /// <summary>
    /// Latest-per-source observations for a single <c>person_id</c> within
    /// the tenant. Empty list when the person has no observations.
    /// </summary>
    Task<IReadOnlyList<PersonObservation>> GetLatestObservationsAsync(
        Guid tenantId,
        Guid personId,
        CancellationToken cancellationToken);

    /// <summary>
    /// Direct subordinates: <c>person_id</c>s whose latest
    /// <c>parent_person_id</c> observation across sources equals
    /// <paramref name="parentPersonId"/>. Reserved for Phase 2; Phase 1
    /// callers ignore the result.
    /// </summary>
    Task<IReadOnlyList<Guid>> GetDirectSubordinateIdsAsync(
        Guid tenantId,
        Guid parentPersonId,
        CancellationToken cancellationToken);

    /// <summary>
    /// Phase 2 (POST /v1/profiles, value_type='email'): distinct
    /// <c>person_id</c>s whose CURRENT email observation on any source
    /// equals <paramref name="emailLowercase"/>. Empty list = no match.
    /// Count &gt; 1 = data invariant violated, caller maps to 422.
    /// </summary>
    Task<IReadOnlyList<Guid>> ResolvePersonIdsByEmailAsync(
        Guid tenantId,
        string emailLowercase,
        CancellationToken cancellationToken);

    /// <summary>
    /// Phase 2 (POST /v1/profiles, value_type='id'): distinct
    /// <c>person_id</c>s whose CURRENT <c>value_type='id'</c>
    /// observation within the given source instance equals
    /// <paramref name="value"/>. Empty list = no match. Count &gt; 1 =
    /// data invariant violated, caller maps to 422.
    /// </summary>
    Task<IReadOnlyList<Guid>> ResolvePersonIdsBySourceIdAsync(
        Guid tenantId,
        string sourceType,
        Guid sourceId,
        string value,
        CancellationToken cancellationToken);

    /// <summary>
    /// Phase 2 (POST /v1/profiles): all CURRENT source-native ids for
    /// one person, one row per source instance (latest
    /// <c>value_type='id'</c> per (source_type, source_id) partition).
    /// Used to populate the <c>ids[]</c> list in the response.
    /// </summary>
    Task<IReadOnlyList<PersonSourceId>> GetCurrentSourceIdsAsync(
        Guid tenantId,
        Guid personId,
        CancellationToken cancellationToken);
}

/// <summary>
/// One source-native id binding for a person — emitted in the
/// <c>ids[]</c> list of <c>POST /v1/profiles</c> response. Domain-layer
/// shape; Api project re-projects to wire DTO.
/// </summary>
public sealed record PersonSourceId(
    string InsightSourceType,
    Guid InsightSourceId,
    string Value);
