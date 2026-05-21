namespace Insight.Identity.Domain.Services;

/// <summary>
/// Email lookup returning a <see cref="Person"/> with the org-tree
/// drawn from <c>org_chart</c> filtered to <see cref="LookupOptions.OrgChartSourceType"/>.
/// </summary>
public sealed class PersonLookupService
{
    private readonly IPersonsReader _reader;

    public PersonLookupService(IPersonsReader reader)
    {
        _reader = reader;
    }

    /// <summary>Lookup by email. Returns <c>null</c> when no current observation matches.</summary>
    public async Task<Person?> GetByEmailAsync(
        Guid tenantId,
        string email,
        LookupOptions options,
        CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(email);
        ArgumentNullException.ThrowIfNull(options);
        // ADR-0011: value_id collation is utf8mb4_unicode_ci, so the SQL
        // comparison handles case. Trim only strips stray whitespace.
        var emailKey = email.Trim();

        var personId = await _reader.ResolvePersonIdByEmailAsync(tenantId, emailKey, cancellationToken)
            .ConfigureAwait(false);
        if (personId is null)
        {
            return null;
        }

        var visited = new HashSet<Guid>();
        // Observations are discarded — only the profile path needs them
        // again (for the ids[] / Profile assembler). GetByEmail returns
        // just the Person.
        var (root, _) = await HydrateAsync(tenantId, personId.Value, options, depth: 0, visited, cancellationToken)
            .ConfigureAwait(false);
        return root;
    }

    /// <summary>
    /// Hydrate the org-tree for a caller that already resolved the
    /// <c>person_id</c>. Returns the assembled <see cref="Person"/>
    /// together with the observations used to build it so the caller
    /// can reuse them without a second DB round-trip.
    /// </summary>
    public async Task<(Person? Person, IReadOnlyList<PersonObservation> Observations)> HydrateForProfileAsync(
        Guid tenantId,
        Guid personId,
        LookupOptions options,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(options);
        var visited = new HashSet<Guid>();
        return await HydrateAsync(tenantId, personId, options, depth: 0, visited, cancellationToken)
            .ConfigureAwait(false);
    }

    private async Task<(Person? Person, IReadOnlyList<PersonObservation> Observations)> HydrateAsync(
        Guid tenantId,
        Guid personId,
        LookupOptions options,
        int depth,
        HashSet<Guid> visited,
        CancellationToken cancellationToken)
    {
        if (!visited.Add(personId))
        {
            return (null, Array.Empty<PersonObservation>());
        }

        var observations = await _reader
            .GetLatestObservationsAsync(tenantId, personId, cancellationToken)
            .ConfigureAwait(false);
        if (observations.Count == 0)
        {
            return (null, Array.Empty<PersonObservation>());
        }

        // Parent is always hydrated when an org_chart edge exists — it
        // is a single O(1) lookup and the legacy parent_* contract is
        // unconditional. Only subordinates recursion is gated by
        // ExpandSubordinates.
        ParentProjection? parent = null;
        var parentEdges = await _reader
            .GetCurrentParentsAsync(tenantId, personId, cancellationToken)
            .ConfigureAwait(false);
        var parentEdge = FilterToSource(parentEdges, options.OrgChartSourceType);
        if (parentEdge is not null)
        {
            parent = await ResolveParentAsync(tenantId, parentEdge, options.OrgChartSourceType, cancellationToken)
                .ConfigureAwait(false);
        }

        IReadOnlyList<Person> subordinates = Array.Empty<Person>();
        if (options.ExpandSubordinates && depth < options.MaxDepth)
        {
            var childEdges = await _reader
                .GetCurrentChildrenAsync(tenantId, personId, cancellationToken)
                .ConfigureAwait(false);
            var childIds = childEdges
                .Where(e => string.Equals(e.InsightSourceType, options.OrgChartSourceType, StringComparison.Ordinal))
                .Select(e => e.ChildPersonId)
                .Distinct()
                .ToList();
            if (childIds.Count > 0)
            {
                var children = new List<Person>(childIds.Count);
                foreach (var childId in childIds)
                {
                    var (built, _) = await HydrateAsync(tenantId, childId, options, depth + 1, visited, cancellationToken)
                        .ConfigureAwait(false);
                    if (built is not null)
                    {
                        children.Add(built);
                    }
                }
                subordinates = children;
            }
        }

        var assembled = PersonAssembler.Assemble(personId, observations, parent, subordinates);
        return (assembled, observations);
    }

    private async Task<ParentProjection> ResolveParentAsync(
        Guid tenantId,
        OrgChartEdge edge,
        string sourceType,
        CancellationToken cancellationToken)
    {
        var parentObservations = await _reader
            .GetLatestObservationsAsync(tenantId, edge.ParentPersonId, cancellationToken)
            .ConfigureAwait(false);

        var latest = parentObservations
            .GroupBy(static o => o.ValueType, StringComparer.Ordinal)
            .ToDictionary(
                static g => g.Key,
                static g => g.OrderByDescending(static o => o.CreatedAt).First().ValueEffective,
                StringComparer.Ordinal);

        var email = latest.GetValueOrDefault(ValueTypes.Email);
        var displayName = latest.GetValueOrDefault(ValueTypes.DisplayName);

        var parentIds = await _reader
            .GetCurrentSourceIdsAsync(tenantId, edge.ParentPersonId, cancellationToken)
            .ConfigureAwait(false);
        var sourceNativeId = parentIds
            .FirstOrDefault(s =>
                string.Equals(s.InsightSourceType, sourceType, StringComparison.Ordinal)
                && s.InsightSourceId == edge.InsightSourceId)
            ?.Value;

        return new ParentProjection(
            PersonId: edge.ParentPersonId,
            Email: email,
            DisplayName: displayName,
            SourceNativeId: sourceNativeId);
    }

    private static OrgChartEdge? FilterToSource(IReadOnlyList<OrgChartEdge> edges, string sourceType)
    {
        for (var i = 0; i < edges.Count; i++)
        {
            if (string.Equals(edges[i].InsightSourceType, sourceType, StringComparison.Ordinal))
            {
                return edges[i];
            }
        }
        return null;
    }
}

/// <summary>
/// Lookup behaviour switches passed from the Api layer into the domain
/// services. <see cref="ExpandSubordinates"/> is the only kill-switch
/// here — parent is always hydrated when an <c>org_chart</c> edge exists.
/// </summary>
public sealed record LookupOptions(
    bool ExpandSubordinates,
    int MaxDepth,
    string OrgChartSourceType)
{
    /// <summary>
    /// Test-only convenience: expand subordinates from BambooHR,
    /// depth-capped at 16. Production paths bind from
    /// <c>AppOptions</c> via <c>PersonsEndpoints.BuildLookupOptions</c>.
    /// </summary>
    public static readonly LookupOptions Default =
        new(ExpandSubordinates: true, MaxDepth: 16, OrgChartSourceType: "bamboohr");
}

/// <summary>Parent edge resolved into the fields the assembler writes onto the response.</summary>
public sealed record ParentProjection(
    Guid PersonId,
    string? Email,
    string? DisplayName,
    string? SourceNativeId);
