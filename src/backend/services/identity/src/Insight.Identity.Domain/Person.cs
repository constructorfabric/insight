namespace Insight.Identity.Domain;

/// <summary>
/// Person projection returned by <c>GET /v1/persons/{email}</c>,
/// including the org-tree (single supervisor + recursive subordinates).
/// </summary>
public sealed record Person(
    Guid PersonId,
    string Email,
    string DisplayName,
    string FirstName,
    string LastName,
    string Department,
    string Division,
    string JobTitle,
    string Status,
    string? SupervisorEmail,
    string? SupervisorName,
    string? ParentEmail,
    string? ParentId,
    Guid? ParentPersonId,
    IReadOnlyList<Person> Subordinates);
