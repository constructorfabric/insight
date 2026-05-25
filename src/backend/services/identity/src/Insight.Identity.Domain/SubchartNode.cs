namespace Insight.Identity.Domain;

/// <summary>
/// One node in the org subchart returned by
/// <c>GET /v1/subchart/{person_id}?depth=N</c>. The minimal node shape —
/// no <c>supervisor_*</c> / <c>parent_*</c> (the parent is the position
/// in the tree), no <c>department</c>/<c>division</c>. Future consumers
/// asking for more fields land via rollforward without breaking the
/// existing contract.
/// </summary>
public sealed record SubchartNode(
    Guid PersonId,
    string? Email,
    string? DisplayName,
    string? JobTitle,
    string? Status,
    IReadOnlyList<SubchartNode> Subordinates);
