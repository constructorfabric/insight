namespace Insight.Identity.Domain.Services;

/// <summary>
/// Phase 2 (cyberfabric/cyber-insight#347) — resolve one profile by
/// email (across all sources) or by source-native id (within one source
/// instance). The data invariant is "at most one current person per
/// lookup"; if more than one matches, the service surfaces
/// <see cref="ProfileLookupResult.Ambiguous"/> and the API layer returns
/// 422 <c>urn:insight:error:ambiguous_profile</c>.
/// </summary>
public sealed class ProfileLookupService
{
    private readonly IPersonsReader _reader;

    public ProfileLookupService(IPersonsReader reader)
    {
        _reader = reader;
    }

    public async Task<ProfileLookupResult> ResolveAsync(
        Guid tenantId,
        ResolveProfileQuery query,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(query);

        IReadOnlyList<Guid> personIds = query.Kind switch
        {
            // No ToLowerInvariant on the email: the storage-layer
            // bugfix in cyberfabric/cyber-insight (ADR-0011 / schema-fix PR)
            // switches `persons.value_id` collation to utf8mb4_unicode_ci,
            // so the comparison is case-insensitive regardless of how
            // the caller typed the email. Trim still strips stray
            // whitespace from URL paths.
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
        var observations = await _reader
            .GetLatestObservationsAsync(tenantId, personId, cancellationToken)
            .ConfigureAwait(false);
        var ids = await _reader
            .GetCurrentSourceIdsAsync(tenantId, personId, cancellationToken)
            .ConfigureAwait(false);

        if (observations.Count == 0)
        {
            // Inconsistent state: resolver returned a person_id but the
            // hydration query found zero rows. Treat as not-found rather
            // than synthesise a hollow profile.
            return new ProfileLookupResult.NotFound();
        }

        var profile = ProfileAssembler.Assemble(personId, tenantId, observations, ids);
        return new ProfileLookupResult.Found(profile);
    }
}

/// <summary>Domain-side request for <see cref="ProfileLookupService"/>.</summary>
public sealed record ResolveProfileQuery(
    ResolveProfileKind Kind,
    string Value,
    string? SourceType,
    Guid? SourceId);

public enum ResolveProfileKind
{
    Email,
    SourceId,
}

/// <summary>
/// Tagged union returned by <see cref="ProfileLookupService.ResolveAsync"/>.
/// </summary>
public abstract record ProfileLookupResult
{
    private ProfileLookupResult() { }

    public sealed record NotFound() : ProfileLookupResult;
    public sealed record Ambiguous(IReadOnlyList<Guid> PersonIds) : ProfileLookupResult;
    public sealed record Found(Profile Profile) : ProfileLookupResult;
}
