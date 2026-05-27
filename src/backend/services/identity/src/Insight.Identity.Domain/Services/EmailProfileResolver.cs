namespace Insight.Identity.Domain.Services;

/// <summary>
/// The email-only profile resolver (the first and, for now, only
/// resolver — see the IR resolver vision). Groups profiles that share
/// the same current email into one <see cref="ProfileGroup"/>. Profiles
/// with no email are returned as singleton groups so the assignment
/// stage can skip them. A future <c>IProfileResolver</c> interface will
/// land when a second resolver (graph / similarity) appears — until
/// then this stays a plain static function (YAGNI).
/// </summary>
public static class EmailProfileResolver
{
    /// <summary>
    /// Group profiles by current email, case-insensitively. Profiles
    /// without an email each become their own singleton group. The
    /// email value is not mutated — case-insensitivity comes from the
    /// <see cref="StringComparer.OrdinalIgnoreCase"/> dictionary, mirroring
    /// the <c>utf8mb4_unicode_ci</c> collation used on the SQL side
    /// (ADR-0011).
    /// </summary>
    public static IReadOnlyList<ProfileGroup> Group(IReadOnlyList<SeedProfile> profiles)
    {
        ArgumentNullException.ThrowIfNull(profiles);

        var byEmail = new Dictionary<string, List<SeedProfile>>(StringComparer.OrdinalIgnoreCase);
        var noEmail = new List<ProfileGroup>();

        foreach (var profile in profiles)
        {
            if (string.IsNullOrEmpty(profile.LatestEmail))
            {
                noEmail.Add(new ProfileGroup(new[] { profile }));
                continue;
            }
            if (!byEmail.TryGetValue(profile.LatestEmail, out var bucket))
            {
                bucket = new List<SeedProfile>();
                byEmail[profile.LatestEmail] = bucket;
            }
            bucket.Add(profile);
        }

        var groups = new List<ProfileGroup>(byEmail.Count + noEmail.Count);
        foreach (var bucket in byEmail.Values)
        {
            groups.Add(new ProfileGroup(bucket));
        }
        groups.AddRange(noEmail);
        return groups;
    }
}
