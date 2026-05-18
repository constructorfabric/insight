using System.Text.Json.Serialization;

namespace Insight.Identity.Api.Contracts;

/// <summary>
/// One source-native account id bound to a person — last observed
/// (`value_type='id'`, latest per (tenant, person, source_type, source_id)).
/// </summary>
public sealed record ProfileIdEntry(
    [property: JsonPropertyName("insight_source_type")] string InsightSourceType,
    [property: JsonPropertyName("insight_source_id")] Guid InsightSourceId,
    [property: JsonPropertyName("value")] string Value);
