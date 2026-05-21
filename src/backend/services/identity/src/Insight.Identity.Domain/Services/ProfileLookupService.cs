namespace Insight.Identity.Domain.Services;

/// <summary>
/// Resolve one profile by email (across all sources) or by source-
/// native id (within one source instance). The data invariant is "at
/// most one current person per lookup"; if more than one matches,
/// the service surfaces <see cref="ProfileLookupResult.Ambiguous"/>
/// and the API layer returns 422 <c>urn:insight:error:ambiguous_profile</c>.
/// Org-tree hydration is delegated to <see cref="PersonLookupService.HydrateForProfileAsync"/>
/// so both endpoints emit identical tree shapes.
/// </summary>
public sealed class ProfileLookupService
{
    private readonly IPersonsReader _reader;
    private readonly PersonLookupService _personLookup;

    public ProfileLookupService(IPersonsReader reader, PersonLookupService personLookup)
    {
        _reader = reader;
        _personLookup = personLookup;
    }

    /// <summary>Lookup. Returns <see cref="ProfileLookupResult.NotFound"/>, <see cref="ProfileLookupResult.Ambiguous"/>, or <see cref="ProfileLookupResult.Found"/>.</summary>
    public async Task<ProfileLookupResult> ResolveAsync(
        Guid tenantId,
        ResolveProfileQuery query,
        LookupOptions options,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(query);
        ArgumentNullException.ThrowIfNull(options);

        // ADR-0011: value_id collation handles case, no client-side
        // normalisation needed.
        IReadOnlyList<Guid> personIds = query.Kind switch
        {
            ResolveProfileKind.Email => await _reader.ResolvePersonIdsByEmailAsync(
                    tenantId,
                    query.Value.Trim(),
                    cancellationToken)
                .ConfigureAwait(false),
            ResolveProfileKind.SourceId => await _reader.ResolvePersonIdsBySourceIdAsync(
                    tenantId,
                    query.SourceType ?? throw new InvalidOperationException("source_type required for value_type='id'"),
                    query.SourceId ?? throw new InvalidOperationException("source_id required for value_type='id'"),
                    query.Value,
                    cancellationToken)
                .ConfigureAwait(false),
            _ => throw new ArgumentOutOfRangeException(nameof(query), query.Kind, "Unknown ResolveProfileKind"),
        };

        if (personIds.Count == 0)
        {
            return new ProfileLookupResult.NotFound();
        }
        if (personIds.Count > 1)
        {
            return new ProfileLookupResult.Ambiguous(personIds);
        }

        var personId = personIds[0];

        var (person, observations) = await _personLookup
            .HydrateForProfileAsync(tenantId, personId, options, cancellationToken)
            .ConfigureAwait(false);
        if (person is null)
        {
            // Resolver returned a person_id but the hydration query
            // found zero rows. Treat as not-found.
            return new ProfileLookupResult.NotFound();
        }

        var ids = await _reader
            .GetCurrentSourceIdsAsync(tenantId, personId, cancellationToken)
            .ConfigureAwait(false);

        var profile = ProfileAssembler.Assemble(personId, tenantId, observations, person, ids);
        return new ProfileLookupResult.Found(profile);
    }
}

/// <summary>Domain-side request for <see cref="ProfileLookupService.ResolveAsync"/>.</summary>
public sealed record ResolveProfileQuery(
    ResolveProfileKind Kind,
    string Value,
    string? SourceType,
    Guid? SourceId);

/// <summary>Selects which resolver path the profile lookup uses.</summary>
public enum ResolveProfileKind
{
    Email,
    SourceId,
}

/// <summary>Tagged union returned by <see cref="ProfileLookupService.ResolveAsync"/>.</summary>
public abstract record ProfileLookupResult
{
    private ProfileLookupResult() { }

    public sealed record NotFound() : ProfileLookupResult;
    public sealed record Ambiguous(IReadOnlyList<Guid> PersonIds) : ProfileLookupResult;
    public sealed record Found(Profile Profile) : ProfileLookupResult;
}
