namespace Insight.Identity.Domain.Services;

/// <summary>
/// Decides "can the caller see this person?" by composing the cheap
/// self check with the recursive-CTE visible-set probe from
/// <see cref="IVisibilityReader"/>. Used by the gating layer in front
/// of <c>/v1/persons</c> and <c>POST /v1/profiles</c>.
/// </summary>
public sealed class VisibilityService
{
    private readonly IVisibilityReader _visibility;

    public VisibilityService(IVisibilityReader visibility)
    {
        _visibility = visibility;
    }

    /// <summary>
    /// Returns <c>true</c> when <paramref name="viewerPersonId"/> is
    /// allowed to see <paramref name="targetPersonId"/> within
    /// <paramref name="tenantId"/>. Identity case (viewer == target)
    /// short-circuits without a DB call; everything else goes through
    /// the visibility CTE bound to <paramref name="orgChartSourceType"/>.
    /// </summary>
    public Task<bool> CanSeeAsync(
        Guid tenantId,
        Guid viewerPersonId,
        Guid targetPersonId,
        string orgChartSourceType,
        CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrEmpty(orgChartSourceType);
        if (viewerPersonId == targetPersonId)
        {
            return Task.FromResult(true);
        }
        return _visibility.IsTargetInVisibleSetAsync(
            tenantId, viewerPersonId, targetPersonId, orgChartSourceType, cancellationToken);
    }
}
