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
    Task<IReadOnlyList<Visibility>> GetActiveVisibilityGrantsByViewerAsync(
        Guid tenantId,
        Guid viewerPersonId,
        CancellationToken cancellationToken);

    /// <summary>
    /// "Is the target in the viewer's visible set?" Runs a single
    /// recursive CTE that unions the viewer with the active
    /// `viewed_person_id`s of their grants (and the target itself
    /// when a whole-tenant grant exists), then walks <c>org_chart</c>
    /// descents filtered to <paramref name="orgChartSourceType"/>.
    /// Self check (viewer == target) is callers' responsibility — see
    /// <see cref="VisibilityService"/>.
    /// </summary>
    Task<bool> IsTargetInVisibleSetAsync(
        Guid tenantId,
        Guid viewerPersonId,
        Guid targetPersonId,
        string orgChartSourceType,
        CancellationToken cancellationToken);

    /// <summary>
    /// One row by <c>visibility_id</c> within the tenant, or
    /// <c>null</c>. Tenant scoping is part of the predicate so a
    /// caller in tenant A cannot read a row from tenant B by id.
    /// </summary>
    Task<Visibility?> GetByIdAsync(Guid tenantId, Guid visibilityId, CancellationToken cancellationToken);

    /// <summary>
    /// Paged list of visibility rows in a tenant, newest first. Use
    /// <paramref name="filterByViewer"/> / <paramref name="filterByViewed"/>
    /// for the canonical "list grants for X" queries; <paramref name="activeOnly"/>
    /// restricts to <c>valid_to IS NULL</c>.
    /// </summary>
    Task<PagedResult<Visibility>> ListAsync(
        Guid tenantId,
        Guid? filterByViewer,
        Guid? filterByViewed,
        bool activeOnly,
        PageRequest page,
        CancellationToken cancellationToken);
}

/// <summary>
/// One row of the <c>visibility</c> table. <see cref="ViewedPersonId"/>
/// is <c>null</c> for a whole-tenant-tree grant.
/// </summary>
public sealed record Visibility(
    Guid VisibilityId,
    Guid InsightTenantId,
    Guid ViewerPersonId,
    Guid? ViewedPersonId,
    DateTime ValidFrom,
    DateTime? ValidTo,
    Guid AuthorPersonId,
    string? Reason,
    DateTime CreatedAt);
