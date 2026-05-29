using Insight.Identity.Domain;

namespace Insight.Identity.Api.Contracts;

/// <summary>
/// Wire-format projection of a <see cref="SubchartNode"/> returned by
/// <c>GET /v1/subchart/{person_id}?depth=N</c>. Property names serialise
/// in snake_case via the project-wide <c>SnakeCaseLower</c> policy.
/// Null fields are emitted as JSON null so consumers can distinguish
/// "no observation" from "missing key".
/// </summary>
public sealed record SubchartNodeResponse(
    Guid PersonId,
    string? Email,
    string? DisplayName,
    string? JobTitle,
    string? Status,
    IReadOnlyList<SubchartNodeResponse> Subordinates)
{
    public static SubchartNodeResponse From(SubchartNode node)
    {
        ArgumentNullException.ThrowIfNull(node);
        var subs = node.Subordinates.Count == 0
            ? Array.Empty<SubchartNodeResponse>()
            : node.Subordinates.Select(From).ToArray();
        return new SubchartNodeResponse(
            node.PersonId,
            node.Email,
            node.DisplayName,
            node.JobTitle,
            node.Status,
            subs);
    }
}

/// <summary>
/// Wrapper for <see cref="SubchartNodeResponse"/> — the outer
/// <c>{ "root": { ... } }</c> shape locked in by the original
/// #348 acceptance criteria so the response is forward-compatible
/// with sibling fields (e.g. depth-cap echoes, pagination hints).
/// </summary>
public sealed record SubchartResponse(SubchartNodeResponse Root);

/// <summary>
/// Forest response for <c>GET /v1/subchart?depth=N</c> (no person id) —
/// #344 follow-up. Returns the trees the caller can see; the array is
/// empty when the caller has no visible-in-source members. Orphans
/// (root with zero subordinates) are filtered out at the service layer
/// per team decision.
/// </summary>
public sealed record SubchartForestResponse(IReadOnlyList<SubchartNodeResponse> Roots);
