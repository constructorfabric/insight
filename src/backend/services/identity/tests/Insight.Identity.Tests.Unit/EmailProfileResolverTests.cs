using FluentAssertions;
using Insight.Identity.Domain.Services;
using Xunit;

namespace Insight.Identity.Tests.Unit;

public sealed class EmailProfileResolverTests
{
    private static readonly Guid SourceId = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");

    private static SeedProfile Profile(string accountId, string? email)
        => new(
            new SourceAccountKey("bamboohr", SourceId, accountId),
            Observations: Array.Empty<IdentityInputRow>(),
            LatestEmail: email,
            IsClosed: false);

    [Fact]
    public void Groups_profiles_sharing_an_email()
    {
        var profiles = new[]
        {
            Profile("acc-1", "a@x.io"),
            Profile("acc-2", "a@x.io"),
            Profile("acc-3", "b@x.io"),
        };

        var groups = EmailProfileResolver.Group(profiles);

        groups.Should().HaveCount(2);
        groups.Should().ContainSingle(g => g.Profiles.Count == 2);
        groups.Should().ContainSingle(g => g.Profiles.Count == 1);
    }

    [Fact]
    public void Groups_emails_case_insensitively()
    {
        // Same email in different case must land in one group — the
        // value is not normalised, the comparer is case-insensitive.
        var profiles = new[]
        {
            Profile("acc-1", "Boss@X.io"),
            Profile("acc-2", "boss@x.io"),
        };

        var groups = EmailProfileResolver.Group(profiles);

        groups.Should().ContainSingle();
        groups[0].Profiles.Should().HaveCount(2);
    }

    [Fact]
    public void No_email_profiles_become_singletons()
    {
        var profiles = new[]
        {
            Profile("acc-1", null),
            Profile("acc-2", null),
        };

        var groups = EmailProfileResolver.Group(profiles);

        groups.Should().HaveCount(2);
        groups.Should().OnlyContain(g => g.Profiles.Count == 1);
    }
}
