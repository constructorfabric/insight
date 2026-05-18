namespace Insight.Identity.Domain;

/// <summary>
/// Domain shape returned by <c>POST /v1/profiles</c> — the latest
/// projection of a person's identity assembled across all sources.
/// Unlike <see cref="Person"/> (Phase 1 GET response):
/// <list type="bullet">
///   <item>Optional fields are nullable rather than empty strings — the
///         API layer hides nulls from JSON via
///         <c>JsonIgnore.WhenWritingNull</c>.</item>
///   <item>Carries <c>InsightTenantId</c> so the caller can confirm the
///         scope of the resolved record.</item>
///   <item>Carries <see cref="Ids"/> — full list of source-native id
///         bindings, one per source instance.</item>
///   <item>No subordinates list (Phase 2 org-tree is a separate
///         endpoint, see cyberfabric/cyber-insight#348).</item>
/// </list>
/// </summary>
public sealed record Profile(
    Guid PersonId,
    Guid InsightTenantId,
    string? Email,
    string? DisplayName,
    string? FirstName,
    string? LastName,
    string? Department,
    string? Division,
    string? JobTitle,
    string? Status,
    string? Username,
    string? EmployeeId,
    string? ParentEmail,
    string? ParentId,
    Guid? ParentPersonId,
    IReadOnlyList<Services.PersonSourceId> Ids);
