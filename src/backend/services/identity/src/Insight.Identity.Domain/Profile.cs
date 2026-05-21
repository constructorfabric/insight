namespace Insight.Identity.Domain;

/// <summary>
/// Domain shape returned by <c>POST /v1/profiles</c> — the latest
/// projection of a person's identity assembled across all sources.
/// Differs from <see cref="Person"/> (the <c>GET /v1/persons/{email}</c>
/// shape):
/// <list type="bullet">
///   <item>Optional fields are nullable rather than empty strings — the
///         API layer hides nulls from JSON via
///         <c>JsonIgnore.WhenWritingNull</c>.</item>
///   <item>Carries <c>InsightTenantId</c> so the caller can confirm the
///         scope of the resolved record.</item>
///   <item>Carries <see cref="Ids"/> — full list of source-native id
///         bindings, one per source instance.</item>
///   <item>The org-tree fields (<see cref="SupervisorEmail"/>,
///         <see cref="SupervisorName"/>, <see cref="Subordinates"/>,
///         legacy <c>parent_*</c>) follow the same single-source walk
///         <see cref="Person"/> uses; both endpoints emit identical
///         tree shapes for the same resolved person.</item>
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
    string? SupervisorEmail,
    string? SupervisorName,
    string? ParentEmail,
    string? ParentId,
    Guid? ParentPersonId,
    IReadOnlyList<Person> Subordinates,
    IReadOnlyList<Services.PersonSourceId> Ids);
