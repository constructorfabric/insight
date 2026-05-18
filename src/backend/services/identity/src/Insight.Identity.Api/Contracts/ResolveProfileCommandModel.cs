using System.Text.Json.Serialization;

namespace Insight.Identity.Api.Contracts;

/// <summary>
/// Body of <c>POST /v1/profiles</c>. Two valid shapes selected by
/// <see cref="ValueType"/>:
/// <list type="bullet">
///   <item>
///     <c>value_type="email"</c> — match across all sources for the tenant.
///     <c>insight_source_type</c> and <c>insight_source_id</c> MUST be null.
///   </item>
///   <item>
///     <c>value_type="id"</c> — match a source-native account id within one
///     source instance. Both <c>insight_source_type</c> and
///     <c>insight_source_id</c> MUST be supplied.
///   </item>
/// </list>
/// The handler resolves to exactly one <c>person_id</c>; multiple matches
/// surface as <c>422 urn:insight:error:ambiguous_profile</c>.
/// </summary>
public sealed record ResolveProfileCommandModel(
    [property: JsonPropertyName("value_type")] string? ValueType,
    [property: JsonPropertyName("value")] string? Value,
    [property: JsonPropertyName("insight_source_type")] string? InsightSourceType,
    [property: JsonPropertyName("insight_source_id")] Guid? InsightSourceId);
