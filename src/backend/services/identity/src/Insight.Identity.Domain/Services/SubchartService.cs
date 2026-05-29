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

    /// <summary>
    /// Forest variant (#344 follow-up): returns every root the
    /// <paramref name="viewerPersonId"/> can see — there is no
    /// caller-chosen root, the set of tops follows from the caller's
    /// <see cref="VisibilityService"/> set and the source's org_chart.
    /// </summary>
    /// <remarks>
    /// <para>
    /// <b>Auth model.</b> Open to every authenticated caller; what they
    /// see depends on their visibility grants:
    /// <list type="bullet">
    ///   <item>No grants → one tree rooted at the caller (their own
    ///   subordinates).</item>
    ///   <item>Grant on a peer → two trees (self + peer).</item>
    ///   <item>Grant on a manager → one tree rooted at the manager
    ///   (the caller falls inside it).</item>
    ///   <item>Wildcard grant → every real top of the source's forest.</item>
    /// </list>
    /// Returns an empty list when the caller has no visible source
    /// membership — never <c>null</c>, never 404.
    /// </para>
    /// <para>
    /// <b>Orphan filter.</b> Roots whose tree consists of just
    /// themselves (no children anywhere in the source's org_chart) are
    /// dropped at the SQL layer — so <c>depth=0</c> still returns legit
    /// tops with an empty <c>subordinates</c> array, instead of every
    /// root being filtered as orphan because descent was bounded. By
    /// team decision: the org_chart still stores the no-parent row
    /// (useful data-quality signal — a person sitting in BambooHR
    /// without a manager assigned), but the endpoint does not surface
    /// them to clients as single-node "trees".
    /// </para>
    /// </remarks>
    public async Task<IReadOnlyList<SubchartNode>> GetForestAsync(
        Guid tenantId,
        Guid viewerPersonId,
        string orgChartSourceType,
        int? maxDepth,
        CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrEmpty(orgChartSourceType);

        var flat = await _reader
            .GetForestAsync(tenantId, viewerPersonId, orgChartSourceType, maxDepth, cancellationToken)
            .ConfigureAwait(false);
        if (flat.Count == 0)
        {
            return Array.Empty<SubchartNode>();
        }

        // Same flat → tree logic as the single-root path. Roots arrive
        // with ParentPersonId == null (the forest SQL projects them as
        // such regardless of their stored row).
        var byParent = new Dictionary<Guid, List<SubchartFlatNode>>(flat.Count);
        var roots = new List<SubchartFlatNode>();
        foreach (var row in flat)
        {
            if (row.ParentPersonId is null)
            {
                roots.Add(row);
                continue;
            }
            if (!byParent.TryGetValue(row.ParentPersonId.Value, out var siblings))
            {
                siblings = new List<SubchartFlatNode>();
                byParent[row.ParentPersonId.Value] = siblings;
            }
            siblings.Add(row);
        }

        // Orphan filtering is done at the SQL layer (only roots with at
        // least one current child in org_chart get seeded), so this is
        // a straight build of every flat root into a tree.
        return roots.Select(r => BuildTree(r, byParent)).ToArray();
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
