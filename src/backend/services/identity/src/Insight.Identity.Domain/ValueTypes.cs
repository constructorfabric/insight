namespace Insight.Identity.Domain;

/// <summary>
/// Canonical <c>value_type</c> taxonomy stored in the <c>persons</c> table.
/// </summary>
/// <remarks>
/// The DB column is a free-form <c>VARCHAR(50)</c> and is intentionally
/// extensible — these constants enumerate the subset this service knows
/// how to project onto the response. Unknown <c>value_type</c>s are
/// read but not surfaced. Relationship fields (<c>parent_*</c>) are not
/// in this set: org-tree edges are sourced from <c>org_chart</c>, not
/// from observations.
/// </remarks>
public static class ValueTypes
{
    public const string Email = "email";
    public const string DisplayName = "display_name";
    public const string FirstName = "first_name";
    public const string LastName = "last_name";
    public const string Department = "department";
    public const string Division = "division";
    public const string JobTitle = "job_title";
    public const string Status = "status";
    public const string EmployeeId = "employee_id";
    public const string Username = "username";
}
