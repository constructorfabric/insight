namespace Insight.Identity.Domain.Services;

/// <summary>
/// Assembles a <see cref="Person"/> from the latest-per-source
/// observations associated with a single <c>person_id</c>. The
/// "latest-per-source" projection is performed by the repository (SQL
/// <c>ROW_NUMBER() OVER PARTITION BY</c>); this class only collapses
/// the multi-source view into a single response shape.
/// </summary>
/// <remarks>
/// <para>
/// <b>Conflict resolution.</b> Two sources may carry different values
/// for the same field. The current implementation picks the most
/// recently observed value across sources (max <c>created_at</c>).
/// Tracked in ADR-0003.
/// </para>
/// <para>
/// <b>Display-name fallback.</b> When neither <c>first_name</c> nor
/// <c>last_name</c> observations exist, the assembler falls back to
/// <see cref="DisplayNameSplitter"/> on the resolved display name.
/// </para>
/// <para>
/// <b>Org-tree.</b> Both <c>supervisor_*</c> and the legacy
/// <c>parent_*</c> triple are filled from the supplied
/// <see cref="ParentProjection"/> (already filtered to the configured
/// org-tree source). A null projection leaves all parent fields null;
/// stale <c>value_type='parent_*'</c> observations on
/// <c>persons</c> are deliberately ignored — <c>org_chart</c> is the
/// sole source for relationships.
/// </para>
/// </remarks>
public static class PersonAssembler
{
    /// <summary>Returns <c>null</c> when no observations are supplied.</summary>
    public static Person? Assemble(
        Guid personId,
        IReadOnlyCollection<PersonObservation> observations,
        ParentProjection? parent,
        IReadOnlyList<Person> subordinates)
    {
        if (observations.Count == 0)
        {
            return null;
        }

        var latest = observations
            .GroupBy(static o => o.ValueType, StringComparer.Ordinal)
            .ToDictionary(
                static g => g.Key,
                static g => g.OrderByDescending(static o => o.CreatedAt).First().ValueEffective,
                StringComparer.Ordinal);

        var email = latest.GetValueOrDefault(ValueTypes.Email, string.Empty);
        var displayName = latest.GetValueOrDefault(ValueTypes.DisplayName, string.Empty);

        var firstName = latest.GetValueOrDefault(ValueTypes.FirstName, string.Empty);
        var lastName = latest.GetValueOrDefault(ValueTypes.LastName, string.Empty);

        if (string.IsNullOrWhiteSpace(firstName) && string.IsNullOrWhiteSpace(lastName)
            && !string.IsNullOrWhiteSpace(displayName))
        {
            (firstName, lastName) = DisplayNameSplitter.Split(displayName);
        }

        return new Person(
            PersonId: personId,
            Email: email,
            DisplayName: displayName,
            FirstName: firstName,
            LastName: lastName,
            Department: latest.GetValueOrDefault(ValueTypes.Department, string.Empty),
            Division: latest.GetValueOrDefault(ValueTypes.Division, string.Empty),
            JobTitle: latest.GetValueOrDefault(ValueTypes.JobTitle, string.Empty),
            Status: latest.GetValueOrDefault(ValueTypes.Status, string.Empty),
            SupervisorEmail: parent?.Email,
            SupervisorName: parent?.DisplayName,
            ParentEmail: parent?.Email,
            ParentId: parent?.SourceNativeId,
            ParentPersonId: parent?.PersonId,
            Subordinates: subordinates);
    }
}
