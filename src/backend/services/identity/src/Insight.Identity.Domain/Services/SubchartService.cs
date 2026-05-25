namespace Insight.Identity.Domain.Services;

/// <summary>
/// Builds the depth-bounded org subchart rooted at a person, gated on
/// visibility to the calling viewer (#348 Phase 3).
/// </summary>
/// <remarks>
/// <para>
/// <b>Visibility model.</b> The gate runs <see cref="VisibilityService.CanSeeAsync"/>
/// against the root only, not per node. The visibility CTE in
/// <see cref="IVisibilityReader"/> is closed under <c>org_chart</c>
/// descent — once the viewer can see the root, every descendant is
/// already in the viewer's visible set. This matches the Phase-2
/// behaviour of the <c>subordinates[]</c> field on <c>GET /v1/persons/{email}</c>
/// and <c>POST /v1/profiles</c>. If the visibility model later gains a
/// per-person revoke that breaks the closure invariant, bulk per-node
/// filtering would be added here.
/// </para>
/// <para>
/// <b>Depth.</b> <c>maxDepth = null</c> means unlimited (constrained by
/// MariaDB's <c>cte_max_recursion_depth</c> = 1000); cycles cannot occur
/// because <c>org_chart</c> is acyclic by construction (the rebuild
/// step in the Python seeder emits a WARN on any 2-hop cycle). DoS via
/// payload size is a gateway-layer concern, not this endpoint's.
/// </para>
/// </remarks>
public sealed class SubchartService
{
    private readonly ISubchartReader _reader;
    private readonly VisibilityService _visibility;

    public SubchartService(ISubchartReader reader, VisibilityService visibility)
    {
        _reader = reader;
        _visibility = visibility;
    }

    /// <summary>
    /// Returns the assembled subchart, or <c>null</c> when the caller
    /// cannot see the root (the API layer maps that to 404 so existence
    /// does not leak).
    /// </summary>
    public async Task<SubchartNode?> GetSubchartAsync(
        Guid tenantId,
        Guid viewerPersonId,
        Guid rootPersonId,
        string orgChartSourceType,
        int? maxDepth,
        CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrEmpty(orgChartSourceType);

        var canSeeRoot = await _visibility
            .CanSeeAsync(tenantId, viewerPersonId, rootPersonId, orgChartSourceType, cancellationToken)
            .ConfigureAwait(false);
        if (!canSeeRoot)
        {
            return null;
        }

        var flat = await _reader
            .GetSubchartAsync(tenantId, rootPersonId, orgChartSourceType, maxDepth, cancellationToken)
            .ConfigureAwait(false);
        if (flat.Count == 0)
        {
            return null;
        }

        // Index rows by parent so the tree build is O(N).
        var byParent = new Dictionary<Guid, List<SubchartFlatNode>>(flat.Count);
        SubchartFlatNode? root = null;
        foreach (var row in flat)
        {
            if (row.ParentPersonId is null)
            {
                root ??= row;
                continue;
            }
            if (!byParent.TryGetValue(row.ParentPersonId.Value, out var siblings))
            {
                siblings = new List<SubchartFlatNode>();
                byParent[row.ParentPersonId.Value] = siblings;
            }
            siblings.Add(row);
        }

        if (root is null)
        {
            return null;
        }
        return BuildTree(root, byParent);
    }

    private static SubchartNode BuildTree(
        SubchartFlatNode node,
        IReadOnlyDictionary<Guid, List<SubchartFlatNode>> byParent)
    {
        IReadOnlyList<SubchartNode> children = Array.Empty<SubchartNode>();
        if (byParent.TryGetValue(node.PersonId, out var rows))
        {
            children = rows.Select(c => BuildTree(c, byParent)).ToArray();
        }
        return new SubchartNode(
            node.PersonId,
            node.Email,
            node.DisplayName,
            node.JobTitle,
            node.Status,
            children);
    }
}
