namespace Insight.Identity.Domain.Services;

/// <summary>
/// Read-side port for the depth-bounded subchart query (#348 Phase 3).
/// Implementation runs a single recursive CTE over <c>org_chart</c> +
/// a per-(person, value_type) latest-observation pass over
/// <c>persons</c> in one round-trip and returns the flat list of nodes;
/// the service layer assembles the tree.
/// </summary>
public interface ISubchartReader
{
    /// <summary>
    /// Flat list of nodes in the subtree rooted at <paramref name="rootPersonId"/>,
    /// ordered by depth then person_id. Each row carries the depth from
    /// the root and the parent_person_id (null on the root). When
    /// <paramref name="maxDepth"/> is null, traversal is unbounded
    /// (constrained only by MariaDB's <c>cte_max_recursion_depth</c>);
    /// otherwise depth is strictly less than the cap.
    /// </summary>
    Task<IReadOnlyList<SubchartFlatNode>> GetSubchartAsync(
        Guid tenantId,
        Guid rootPersonId,
        string orgChartSourceType,
        int? maxDepth,
        CancellationToken cancellationToken);
}

/// <summary>
/// One flat row of the subchart CTE result. The service layer turns
/// this list into a tree by indexing on <see cref="ParentPersonId"/>.
/// </summary>
public sealed record SubchartFlatNode(
    Guid PersonId,
    Guid? ParentPersonId,
    int Depth,
    string? Email,
    string? DisplayName,
    string? JobTitle,
    string? Status);
