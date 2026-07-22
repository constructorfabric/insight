using System.Security.Claims;
using FluentAssertions;
using Insight.Identity.Api.Auth;
using Microsoft.AspNetCore.Http;
using Xunit;

namespace Insight.Identity.Tests.Unit;

public sealed class SubjectCallerContextTests
{
    private static readonly Guid Person = Guid.Parse("33333333-3333-4333-8333-333333333333");

    private static DefaultHttpContext WithSub(string? sub)
    {
        var claims = sub is null ? Array.Empty<Claim>() : [new Claim("sub", sub)];
        return new DefaultHttpContext
        {
            User = new ClaimsPrincipal(new ClaimsIdentity(claims, "test")),
        };
    }

    [Fact]
    public async Task Resolves_person_id_from_sub()
    {
        var result = await new SubjectCallerContext()
            .ResolveAsync(WithSub(Person.ToString("D")), CancellationToken.None);
        result.Should().Be(Person);
    }

    [Fact]
    public async Task Service_subject_has_no_person()
    {
        var result = await new SubjectCallerContext()
            .ResolveAsync(WithSub("service:seeder"), CancellationToken.None);
        result.Should().BeNull();
    }

    [Fact]
    public async Task Missing_sub_has_no_person()
    {
        var result = await new SubjectCallerContext()
            .ResolveAsync(WithSub(null), CancellationToken.None);
        result.Should().BeNull();
    }

    [Fact]
    public async Task Non_uuid_sub_has_no_person()
    {
        var result = await new SubjectCallerContext()
            .ResolveAsync(WithSub("not-a-uuid"), CancellationToken.None);
        result.Should().BeNull();
    }
}
