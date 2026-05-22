namespace Insight.Identity.Domain.Services;

/// <summary>
/// Read-side port over the `person_roles` junction. Two patterns served:
/// the hot "is this caller an admin?" predicate used by the future
/// CRUD-endpoint authz filter, and the "list every active role of one
/// person" query that feeds the CRUD response shape.
/// </summary>
public interface IPersonRolesReader
{
    /// <summary>
    /// Single-row probe: does <paramref name="personId"/> currently
    /// hold <paramref name="roleId"/> in <paramref name="tenantId"/>?
    /// "Currently" = there is at least one row with <c>valid_to IS NULL</c>
    /// matching the triple. Used by the admin authz gate.
    /// </summary>
    Task<bool> HasActiveRoleAsync(
        Guid tenantId,
        Guid personId,
        Guid roleId,
        CancellationToken cancellationToken);

    /// <summary>
    /// All active role grants for one person in one tenant. Empty list
    /// when the person has no roles.
    /// </summary>
    Task<IReadOnlyList<PersonRole>> GetActiveByPersonAsync(
        Guid tenantId,
        Guid personId,
        CancellationToken cancellationToken);
}

/// <summary>One row of the <c>person_roles</c> junction.</summary>
public sealed record PersonRole(
    Guid PersonRoleId,
    Guid InsightTenantId,
    Guid PersonId,
    Guid RoleId,
    DateTime ValidFrom,
    DateTime? ValidTo,
    Guid AuthorPersonId,
    string? Reason,
    DateTime CreatedAt);
