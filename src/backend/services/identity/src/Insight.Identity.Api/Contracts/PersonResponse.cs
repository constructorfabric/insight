using System.Text.Json.Serialization;
using Insight.Identity.Domain;

namespace Insight.Identity.Api.Contracts;

/// <summary>
/// Wire-format projection of <see cref="Person"/> returned by
/// <c>GET /v1/persons/{email}</c>. Snake-case JSON keeps existing
/// api-gateway adapters compatible.
/// </summary>
public sealed record PersonResponse(
    [property: JsonPropertyName("person_id")] Guid PersonId,
    [property: JsonPropertyName("email")] string Email,
    [property: JsonPropertyName("display_name")] string DisplayName,
    [property: JsonPropertyName("first_name")] string FirstName,
    [property: JsonPropertyName("last_name")] string LastName,
    [property: JsonPropertyName("department")] string Department,
    [property: JsonPropertyName("division")] string Division,
    [property: JsonPropertyName("job_title")] string JobTitle,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("supervisor_email")] string? SupervisorEmail,
    [property: JsonPropertyName("supervisor_name")] string? SupervisorName,
    [property: JsonPropertyName("parent_email")] string? ParentEmail,
    [property: JsonPropertyName("parent_id")] string? ParentId,
    [property: JsonPropertyName("parent_person_id")] Guid? ParentPersonId,
    [property: JsonPropertyName("subordinates")] IReadOnlyList<PersonResponse> Subordinates)
{
    public static PersonResponse From(Person person)
    {
        ArgumentNullException.ThrowIfNull(person);
        var subs = person.Subordinates.Count == 0
            ? Array.Empty<PersonResponse>()
            : person.Subordinates.Select(From).ToArray();
        return new PersonResponse(
            person.PersonId,
            person.Email,
            person.DisplayName,
            person.FirstName,
            person.LastName,
            person.Department,
            person.Division,
            person.JobTitle,
            person.Status,
            person.SupervisorEmail,
            person.SupervisorName,
            person.ParentEmail,
            person.ParentId,
            person.ParentPersonId,
            subs);
    }
}
