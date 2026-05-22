namespace Insight.Identity.Domain.Services;

/// <summary>
/// Read-side port over the <c>visibility</c> table — the `viewer → viewed`
/// SCD2 grant list that, combined with the `org_chart` cache, decides
/// whether one caller can see another person's record. The recursive
/// `can_see(viewer, target)` predicate is built on top of this port by
/// <c>VisibilityService</c>, which joins these seed rows with
/// <c>org_chart</c> at query time.
/// </summary>
public interface IVisibilityReader
{
    /// <summary>
    /// All active grants for one viewer in one tenant. "Active" means
    /// <c>valid_to IS NULL</c>; the returned list is the input the
    /// visibility CTE uses as its seed set together with the viewer's
    /// own <c>person_id</c>.
    /// </summary>
    Task<IReadOnlyList<VisibilityGrant>> GetActiveGrantsByViewerAsync(
        Guid tenantId,
        Guid viewerPersonId,
        CancellationToken cancellationToken);
}

/// <summary>
/// One row of the `visibility` table projected into the domain layer.
/// <see cref="ViewedPersonId"/> is <c>null</c> when the grant covers
/// the whole tenant tree (whole-tree scope).
/// </summary>
public sealed record VisibilityGrant(
    Guid VisibilityId,
    Guid InsightTenantId,
    Guid ViewerPersonId,
    Guid? ViewedPersonId,
    DateTime ValidFrom,
    DateTime? ValidTo,
    Guid AuthorPersonId,
    string Reason,
    DateTime CreatedAt);
