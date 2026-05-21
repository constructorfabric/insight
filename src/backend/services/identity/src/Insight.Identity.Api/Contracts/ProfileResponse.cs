using System.Text.Json.Serialization;
using Insight.Identity.Domain;

namespace Insight.Identity.Api.Contracts;

/// <summary>
/// Response body of <c>POST /v1/profiles</c>. Extends
/// <see cref="PersonResponse"/> with profile-specific fields
/// (<see cref="InsightTenantId"/>, <see cref="Username"/>,
/// <see cref="EmployeeId"/>, and <see cref="Ids"/> — every current
/// <c>value_type='id'</c> observation, one per (source_type,
/// source_id) instance). Null-valued optional fields are omitted
/// from JSON to keep the payload tight.
/// </summary>
public sealed record ProfileResponse(
    [property: JsonPropertyName("person_id")] Guid PersonId,
    [property: JsonPropertyName("insight_tenant_id")] Guid InsightTenantId,
    [property: JsonPropertyName("email")] string? Email,
    [property: JsonPropertyName("display_name")] string? DisplayName,
    [property: JsonPropertyName("first_name"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? FirstName,
    [property: JsonPropertyName("last_name"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? LastName,
    [property: JsonPropertyName("department"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? Department,
    [property: JsonPropertyName("division"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? Division,
    [property: JsonPropertyName("job_title"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? JobTitle,
    [property: JsonPropertyName("status"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? Status,
    [property: JsonPropertyName("username"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? Username,
    [property: JsonPropertyName("employee_id"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? EmployeeId,
    [property: JsonPropertyName("supervisor_email"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? SupervisorEmail,
    [property: JsonPropertyName("supervisor_name"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? SupervisorName,
    [property: JsonPropertyName("parent_email"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? ParentEmail,
    [property: JsonPropertyName("parent_id"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? ParentId,
    [property: JsonPropertyName("parent_person_id"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] Guid? ParentPersonId,
    [property: JsonPropertyName("subordinates")] IReadOnlyList<PersonResponse> Subordinates,
    [property: JsonPropertyName("ids")] IReadOnlyList<ProfileIdEntry> Ids)
{
    public static ProfileResponse From(Profile profile)
    {
        ArgumentNullException.ThrowIfNull(profile);
        var ids = profile.Ids.Count == 0
            ? Array.Empty<ProfileIdEntry>()
            : profile.Ids
                .Select(static s => new ProfileIdEntry(s.InsightSourceType, s.InsightSourceId, s.Value))
                .ToArray();
        var subs = profile.Subordinates.Count == 0
            ? Array.Empty<PersonResponse>()
            : profile.Subordinates.Select(PersonResponse.From).ToArray();
        return new ProfileResponse(
            profile.PersonId,
            profile.InsightTenantId,
            profile.Email,
            profile.DisplayName,
            profile.FirstName,
            profile.LastName,
            profile.Department,
            profile.Division,
            profile.JobTitle,
            profile.Status,
            profile.Username,
            profile.EmployeeId,
            profile.SupervisorEmail,
            profile.SupervisorName,
            profile.ParentEmail,
            profile.ParentId,
            profile.ParentPersonId,
            subs,
            ids);
    }
}
