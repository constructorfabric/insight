using System.Text.Json.Serialization;

namespace Insight.Identity.Api.Contracts;

/// <summary>
/// Specialised RFC 7807 body for <c>422 urn:insight:error:ambiguous_profile</c>.
/// Echoes back the offending lookup and lists the <c>person_id</c>s that
/// matched so the caller can investigate the data-invariant violation
/// without re-querying.
/// </summary>
public sealed record AmbiguousProfileProblemResponse(
    [property: JsonPropertyName("type")] string Type,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("status")] int Status,
    [property: JsonPropertyName("detail")] string Detail,
    [property: JsonPropertyName("lookup")] ResolveProfileCommandModel Lookup,
    [property: JsonPropertyName("person_ids")] IReadOnlyList<Guid> PersonIds);
