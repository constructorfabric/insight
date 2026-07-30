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
    /// <c>value_type='email'</c> = <paramref name="email"/>. The
    /// comparison is case-insensitive thanks to the
    /// <c>utf8mb4_unicode_ci</c> collation on <c>persons.value_id</c>
    /// (ADR-0011).
    /// </summary>
    Task<Guid?> ResolvePersonIdByEmailAsync(
        Guid tenantId,
        string email,
        CancellationToken cancellationToken);

    /// <summary>
    /// Tenant-AGNOSTIC email → <c>person_id</c> resolution for the login
    /// bootstrap: the authenticator's service-only lookup runs before any tenant
    /// is known, so the tenant filter is dropped and any matching tenant's
    /// latest observation wins. Returns <c>null</c> when no current
    /// <c>value_type='email'</c> observation matches. NEVER call this on a
    /// user-facing path — it crosses tenant boundaries by design.
    /// </summary>
    Task<Guid?> ResolvePersonIdByEmailAnyTenantAsync(
        string email,
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
    /// Current parent edges for <paramref name="childPersonId"/> across
    /// all source instances within the tenant. Reads <c>org_chart</c>
    /// rows with <c>valid_to IS NULL</c>; an empty list means the
    /// person has no recorded parent in any source. At most one CURRENT
    /// parent per (tenant, source_type, source_id, child), so the list
    /// size equals the number of source instances that have a current
    /// parent observation for this person.
    /// </summary>
    Task<IReadOnlyList<OrgChartEdge>> GetCurrentParentsAsync(
        Guid tenantId,
        Guid childPersonId,
        CancellationToken cancellationToken);

    /// <summary>
    /// Current direct-children edges where <paramref name="parentPersonId"/>
    /// is the parent. Reads <c>org_chart</c> rows with
    /// <c>valid_to IS NULL</c>; an empty list means no one currently
    /// reports to this person in any source.
    /// </summary>
    // TODO(#348-phase3): the /v1/subchart/{person_id}?depth=N endpoint
    // will build on this query with a depth-bounded recursive CTE.
    Task<IReadOnlyList<OrgChartEdge>> GetCurrentChildrenAsync(
        Guid tenantId,
        Guid parentPersonId,
        CancellationToken cancellationToken);

    /// <summary>
    /// Distinct <c>person_id</c>s whose CURRENT email observation on
    /// any source equals <paramref name="email"/>. Empty list = no match.
    /// Count &gt; 1 = data invariant violated, caller maps to 422.
    /// The comparison is case-insensitive thanks to the
    /// <c>utf8mb4_unicode_ci</c> collation on <c>persons.value_id</c>
    /// (ADR-0011). Backs <c>POST /v1/profiles</c> with
    /// <c>value_type='email'</c>.
    /// </summary>
    Task<IReadOnlyList<Guid>> ResolvePersonIdsByEmailAsync(
        Guid tenantId,
        string email,
        CancellationToken cancellationToken);

    /// <summary>
    /// Distinct <c>person_id</c>s whose CURRENT <c>value_type='id'</c>
    /// observation within the given source instance equals
    /// <paramref name="value"/>. Empty list = no match. Count &gt; 1 =
    /// data invariant violated, caller maps to 422. Backs
    /// <c>POST /v1/profiles</c> with <c>value_type='id'</c>.
    /// </summary>
    Task<IReadOnlyList<Guid>> ResolvePersonIdsBySourceIdAsync(
        Guid tenantId,
        string sourceType,
        Guid sourceId,
        string value,
        CancellationToken cancellationToken);

    /// <summary>
    /// All CURRENT source-native ids for one person, one row per source
    /// instance (latest <c>value_type='id'</c> per (source_type,
    /// source_id) partition). Populates the <c>ids[]</c> list on the
    /// <c>POST /v1/profiles</c> response.
    /// </summary>
    Task<IReadOnlyList<PersonSourceId>> GetCurrentSourceIdsAsync(
        Guid tenantId,
        Guid personId,
        CancellationToken cancellationToken);

    /// <summary>
    /// Returns the <c>person_id</c> bound to a source-native account
    /// id within the tenant (active <c>account_person_map</c> row
    /// only). Used for caller resolution from JWT id claims (oid /
    /// sub). Returns <c>null</c> when no active binding matches.
    /// </summary>
    Task<Guid?> ResolvePersonIdByAccountIdAsync(
        Guid tenantId,
        string accountId,
        CancellationToken cancellationToken);
}

/// <summary>
/// One parent->child edge from <c>org_chart</c>, scoped to a single
/// source instance. The same person may appear as a
/// <c>ChildPersonId</c> in multiple edges, one per source instance
/// where the source emitted a parent observation for them; the edge
/// granularity is therefore (tenant, source_type, source_id, child).
/// </summary>
public sealed record OrgChartEdge(
    string InsightSourceType,
    Guid InsightSourceId,
    Guid ChildPersonId,
    Guid ParentPersonId,
    DateTime ValidFrom);

/// <summary>
/// One source-native id binding for a person — emitted in the
/// <c>ids[]</c> list of the <c>POST /v1/profiles</c> response. Domain-
/// layer shape; Api project re-projects to wire DTO.
/// </summary>
public sealed record PersonSourceId(
    string InsightSourceType,
    Guid InsightSourceId,
    string Value);
