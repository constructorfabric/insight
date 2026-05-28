namespace Insight.Identity.Domain.Services;

/// <summary>
/// Turns resolver-produced <see cref="ProfileGroup"/>s into
/// <see cref="PersonAssignment"/>s — deciding which <c>person_id</c>
/// each group binds to. Mirrors the Python seeder's per-account
/// classification, lifted to the group level:
/// <list type="number">
///   <item>Any profile in the group already bound to a person (known
///   <c>source_account_id</c>) → reuse that person for the whole group
///   (idempotency for re-seeds).</item>
///   <item>Else, the group's email already maps to an existing person
///   in <c>persons</c> → link the group to that person
///   (<c>auto-seed-link</c>).</item>
///   <item>Else, the group has at least one active (non-closed)
///   profile → mint a new person for the group.</item>
///   <item>Else (no binding, no email match, all profiles closed) →
///   skip; closed accounts never create new persons.</item>
/// </list>
/// </summary>
public static class PersonAssignmentResolver
{
    /// <summary>The reason written on observations linked by the email path.</summary>
    public const string AutoSeedLinkReason = "auto-seed-link";

    public sealed record Result(
        IReadOnlyList<PersonAssignment> Assignments,
        int ReusedKnown,
        int LinkedByEmail,
        int MintedNew,
        int SkippedClosed,
        int SkippedNoEmail);

    /// <param name="groups">Resolver output.</param>
    /// <param name="knownAccounts">Current <c>source_account_id → person_id</c> bindings from <c>persons</c> (value_type='id').</param>
    /// <param name="emailToPerson">Current latest <c>email → person_id</c> map from <c>persons</c> (value_type='email'). Keys are raw emails; the dictionary MUST use a case-insensitive comparer (e.g. <see cref="StringComparer.OrdinalIgnoreCase"/>) — emails are stored as-is and matched case-insensitively per ADR-0011.</param>
    /// <param name="mintPersonId">Factory for a fresh person_id (injected so tests are deterministic).</param>
    public static Result Resolve(
        IReadOnlyList<ProfileGroup> groups,
        IReadOnlyDictionary<SourceAccountKey, Guid> knownAccounts,
        IReadOnlyDictionary<string, Guid> emailToPerson,
        Func<Guid> mintPersonId)
    {
        ArgumentNullException.ThrowIfNull(groups);
        ArgumentNullException.ThrowIfNull(knownAccounts);
        ArgumentNullException.ThrowIfNull(emailToPerson);
        ArgumentNullException.ThrowIfNull(mintPersonId);

        var assignments = new List<PersonAssignment>(groups.Count);
        int reusedKnown = 0, linkedByEmail = 0, mintedNew = 0, skippedClosed = 0, skippedNoEmail = 0;

        foreach (var group in groups)
        {
            // 1. Known binding wins — find the first profile already
            //    bound to a person; reuse that person for the group.
            Guid? knownPerson = null;
            foreach (var profile in group.Profiles)
            {
                if (knownAccounts.TryGetValue(profile.Account, out var pid))
                {
                    knownPerson = pid;
                    break;
                }
            }
            if (knownPerson is { } known)
            {
                assignments.Add(new PersonAssignment(known, AssignmentKind.ReusedKnown, group.Profiles));
                reusedKnown += group.Profiles.Count;
                continue;
            }

            // The group's email (every profile in an email group shares
            // it; singleton no-email groups have null).
            var email = group.Profiles[0].LatestEmail;
            if (string.IsNullOrEmpty(email))
            {
                skippedNoEmail += group.Profiles.Count;
                continue;
            }

            // 2. Email matches an existing person → link.
            if (emailToPerson.TryGetValue(email, out var emailPerson))
            {
                assignments.Add(new PersonAssignment(emailPerson, AssignmentKind.LinkedByEmail, group.Profiles));
                linkedByEmail += group.Profiles.Count;
                continue;
            }

            // 3/4. No binding, no email match. Mint only if at least one
            //      profile is active; a wholly-closed group is skipped.
            var hasActive = group.Profiles.Any(p => !p.IsClosed);
            if (!hasActive)
            {
                skippedClosed += group.Profiles.Count;
                continue;
            }
            assignments.Add(new PersonAssignment(mintPersonId(), AssignmentKind.Minted, group.Profiles));
            mintedNew += group.Profiles.Count;
        }

        return new Result(assignments, reusedKnown, linkedByEmail, mintedNew, skippedClosed, skippedNoEmail);
    }
}
