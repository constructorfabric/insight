namespace Insight.Identity.Domain.Services;

/// <summary>
/// Read-side port over the strict-minimum `roles` catalogue. RBAC
/// primitive — currently one row (`admin`); see #346 design rev 3.1.
/// </summary>
public interface IRolesReader
{
    /// <summary>
    /// Resolve a role-by-name lookup. Returns <c>null</c> when the role
    /// is not seeded. Stable across the lifetime of the database; the
    /// caller is expected to cache the result.
    /// </summary>
    Task<Role?> GetByNameAsync(string name, CancellationToken cancellationToken);

    /// <summary>Enumerate every role row.</summary>
    Task<IReadOnlyList<Role>> ListAllAsync(CancellationToken cancellationToken);
}

/// <summary>One `roles` row projected into the domain layer.</summary>
public sealed record Role(Guid RoleId, string Name);

/// <summary>
/// Pinned UUIDs for the seeded built-in roles. The infrastructure
/// migration (<c>007_roles.sql</c>) writes these exact values; code
/// that needs to reference a role by identity uses these constants
/// instead of taking a runtime <see cref="IRolesReader.GetByNameAsync"/>
/// round-trip on every authz check.
/// </summary>
public static class Roles
{
    public static readonly Guid Admin = Guid.Parse("a4d11000-0000-4000-8000-000000000001");
    public const string AdminName = "admin";
}
