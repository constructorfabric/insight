namespace Insight.Identity.Domain.Services;

/// <summary>
/// Assembles a <see cref="Profile"/> from the latest-per-source
/// observations of a single <c>person_id</c>, the hydrated org-tree
/// projection, and the list of current source-native id bindings.
/// The latest-per-source projection is performed in SQL by the
/// repository; this class just collapses the multi-source rows into
/// a single response shape.
/// </summary>
/// <remarks>
/// Differs from <see cref="PersonAssembler"/>: the result uses
/// nullable strings (the API layer drops null fields from JSON
/// instead of emitting empty strings) and ships the <c>ids[]</c>
/// projection alongside the assembled attributes. The org-tree
/// (<c>supervisor_*</c>, <c>parent_*</c>, <c>subordinates[]</c>) is
/// borrowed off the supplied <see cref="Person"/> projection so both
/// endpoints emit identical tree shapes. Conflict resolution is the
/// same as <see cref="PersonAssembler"/> — max <c>created_at</c> wins
/// per <c>value_type</c> (ADR-0003).
/// </remarks>
public static class ProfileAssembler
{
    public static Profile Assemble(
        Guid personId,
        Guid tenantId,
        IReadOnlyCollection<PersonObservation> observations,
        Person orgTree,
        IReadOnlyList<PersonSourceId> ids)
    {
        ArgumentNullException.ThrowIfNull(orgTree);

        var latest = observations
            .GroupBy(static o => o.ValueType, StringComparer.Ordinal)
            .ToDictionary(
                static g => g.Key,
                static g => g.OrderByDescending(static o => o.CreatedAt).First().ValueEffective,
                StringComparer.Ordinal);

        var displayName = NullIfBlank(latest.GetValueOrDefault(ValueTypes.DisplayName));
        var firstName = NullIfBlank(latest.GetValueOrDefault(ValueTypes.FirstName));
        var lastName = NullIfBlank(latest.GetValueOrDefault(ValueTypes.LastName));

        if (firstName is null && lastName is null && displayName is not null)
        {
            (firstName, lastName) = DisplayNameSplitter.Split(displayName);
            firstName = NullIfBlank(firstName);
            lastName = NullIfBlank(lastName);
        }

        return new Profile(
            PersonId: personId,
            InsightTenantId: tenantId,
            Email: NullIfBlank(latest.GetValueOrDefault(ValueTypes.Email)),
            DisplayName: displayName,
            FirstName: firstName,
            LastName: lastName,
            Department: NullIfBlank(latest.GetValueOrDefault(ValueTypes.Department)),
            Division: NullIfBlank(latest.GetValueOrDefault(ValueTypes.Division)),
            JobTitle: NullIfBlank(latest.GetValueOrDefault(ValueTypes.JobTitle)),
            Status: NullIfBlank(latest.GetValueOrDefault(ValueTypes.Status)),
            Username: NullIfBlank(latest.GetValueOrDefault(ValueTypes.Username)),
            EmployeeId: NullIfBlank(latest.GetValueOrDefault(ValueTypes.EmployeeId)),
            SupervisorEmail: NullIfBlank(orgTree.SupervisorEmail),
            SupervisorName: NullIfBlank(orgTree.SupervisorName),
            ParentEmail: NullIfBlank(orgTree.ParentEmail),
            ParentId: NullIfBlank(orgTree.ParentId),
            ParentPersonId: orgTree.ParentPersonId,
            Subordinates: orgTree.Subordinates,
            Ids: ids);
    }

    private static string? NullIfBlank(string? value) =>
        string.IsNullOrWhiteSpace(value) ? null : value;
}
