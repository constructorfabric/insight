using FluentAssertions;
using Insight.Identity.Api.Auth;
using Insight.Identity.Domain;
using Insight.Identity.Domain.Services;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.Logging.Abstractions;
using Xunit;

namespace Insight.Identity.Tests.Unit;

public sealed class HeaderCallerContextTests
{
    private static readonly Guid CallerId = Guid.Parse("33333333-3333-3333-3333-333333333333");

    [Fact]
    public async Task Returns_parsed_guid_when_header_present()
    {
        var context = new DefaultHttpContext();
        context.Request.Headers[HeaderCallerContext.HeaderName] = CallerId.ToString();

        var resolved = await NewSut().ResolveAsync(context, CancellationToken.None);

        resolved.Should().Be(CallerId);
    }

    [Fact]
    public async Task Returns_null_when_header_missing_and_no_jwt_claims()
    {
        var context = new DefaultHttpContext();

        var resolved = await NewSut().ResolveAsync(context, CancellationToken.None);

        resolved.Should().BeNull();
    }

    [Theory]
    [InlineData("")]
    [InlineData("not-a-guid")]
    [InlineData("33333333-3333-3333-3333")]
    public async Task Returns_null_when_header_value_is_not_a_guid(string raw)
    {
        var context = new DefaultHttpContext();
        context.Request.Headers[HeaderCallerContext.HeaderName] = raw;

        var resolved = await NewSut().ResolveAsync(context, CancellationToken.None);

        resolved.Should().BeNull();
    }

    [Fact]
    public async Task Rejects_guid_empty()
    {
        var context = new DefaultHttpContext();
        context.Request.Headers[HeaderCallerContext.HeaderName] = Guid.Empty.ToString();

        // Guid.Empty is parseable but is not a real identity — accepting it
        // would promote `00000000-…` to a valid caller and pollute the
        // audit trail. JWT-fallback also returns null with no claims set.
        var resolved = await NewSut().ResolveAsync(context, CancellationToken.None);

        resolved.Should().BeNull();
    }

    [Fact]
    public async Task Caches_resolved_result_per_request()
    {
        // The resolver memoises on HttpContext.Items so multiple intra-
        // handler probes (visibility check, admin gate, audit) do not
        // re-hit MariaDB. Verified by a counting reader: a second call
        // on the same HttpContext must not invoke the reader again.
        var context = new DefaultHttpContext();
        // No header, no JWT claims — resolution returns null via the
        // null-tenant short-circuit. The cache must still memoise the
        // null result.
        var counting = new CountingReader();
        var sut = new HeaderCallerContext(counting, new NullTenantContext(), NullLogger<HeaderCallerContext>.Instance);

        var first  = await sut.ResolveAsync(context, CancellationToken.None);
        var second = await sut.ResolveAsync(context, CancellationToken.None);

        first.Should().BeNull();
        second.Should().BeNull();
        counting.ResolvePersonIdByAccountIdCalls.Should().Be(0,
            "no tenant means the JWT lookup path never runs; cache test still proves the cache key is present");

        // Now exercise the lookup path: set a tenant + an oid claim that
        // resolves to a real caller, then call twice — counter must be 1.
        var context2 = new DefaultHttpContext();
        var tenant = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
        var person = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        var identity = new System.Security.Claims.ClaimsIdentity("test");
        identity.AddClaim(new System.Security.Claims.Claim("oid", "some-oid"));
        context2.User = new System.Security.Claims.ClaimsPrincipal(identity);
        var counting2 = new CountingReader(returnedPersonId: person);
        var sut2 = new HeaderCallerContext(counting2, new FixedTenantContext(tenant), NullLogger<HeaderCallerContext>.Instance);

        var firstReal  = await sut2.ResolveAsync(context2, CancellationToken.None);
        var secondReal = await sut2.ResolveAsync(context2, CancellationToken.None);

        firstReal.Should().Be(person);
        secondReal.Should().Be(person);
        counting2.ResolvePersonIdByAccountIdCalls.Should().Be(1,
            "the second call must come from HttpContext.Items, not from MariaDB");
    }

    private static HeaderCallerContext NewSut()
        => new(new NullReader(), new NullTenantContext(), NullLogger<HeaderCallerContext>.Instance);

    private sealed class FixedTenantContext(Guid tenantId) : ITenantContext
    {
        public Guid? Resolve(HttpContext context) => tenantId;
    }

    private sealed class CountingReader(Guid? returnedPersonId = null) : IPersonsReader
    {
        public int ResolvePersonIdByAccountIdCalls { get; private set; }

        public Task<Guid?> ResolvePersonIdByAccountIdAsync(Guid tenantId, string accountId, CancellationToken cancellationToken)
        {
            ResolvePersonIdByAccountIdCalls++;
            return Task.FromResult(returnedPersonId);
        }
        public Task<Guid?> ResolvePersonIdByEmailAsync(Guid tenantId, string email, CancellationToken cancellationToken)
            => Task.FromResult<Guid?>(null);
        public Task<IReadOnlyList<PersonObservation>> GetLatestObservationsAsync(Guid tenantId, Guid personId, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<PersonObservation>>(Array.Empty<PersonObservation>());
        public Task<IReadOnlyList<OrgChartEdge>> GetCurrentParentsAsync(Guid tenantId, Guid childPersonId, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<OrgChartEdge>>(Array.Empty<OrgChartEdge>());
        public Task<IReadOnlyList<OrgChartEdge>> GetCurrentChildrenAsync(Guid tenantId, Guid parentPersonId, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<OrgChartEdge>>(Array.Empty<OrgChartEdge>());
        public Task<IReadOnlyList<Guid>> ResolvePersonIdsByEmailAsync(Guid tenantId, string email, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<Guid>>(Array.Empty<Guid>());
        public Task<IReadOnlyList<Guid>> ResolvePersonIdsBySourceIdAsync(Guid tenantId, string sourceType, Guid sourceId, string value, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<Guid>>(Array.Empty<Guid>());
        public Task<IReadOnlyList<PersonSourceId>> GetCurrentSourceIdsAsync(Guid tenantId, Guid personId, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<PersonSourceId>>(Array.Empty<PersonSourceId>());
    }

    private sealed class NullTenantContext : ITenantContext
    {
        public Guid? Resolve(HttpContext context) => null;
    }

    private sealed class NullReader : IPersonsReader
    {
        public Task<Guid?> ResolvePersonIdByEmailAsync(Guid tenantId, string email, CancellationToken cancellationToken)
            => Task.FromResult<Guid?>(null);
        public Task<IReadOnlyList<PersonObservation>> GetLatestObservationsAsync(Guid tenantId, Guid personId, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<PersonObservation>>(Array.Empty<PersonObservation>());
        public Task<IReadOnlyList<OrgChartEdge>> GetCurrentParentsAsync(Guid tenantId, Guid childPersonId, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<OrgChartEdge>>(Array.Empty<OrgChartEdge>());
        public Task<IReadOnlyList<OrgChartEdge>> GetCurrentChildrenAsync(Guid tenantId, Guid parentPersonId, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<OrgChartEdge>>(Array.Empty<OrgChartEdge>());
        public Task<IReadOnlyList<Guid>> ResolvePersonIdsByEmailAsync(Guid tenantId, string email, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<Guid>>(Array.Empty<Guid>());
        public Task<IReadOnlyList<Guid>> ResolvePersonIdsBySourceIdAsync(Guid tenantId, string sourceType, Guid sourceId, string value, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<Guid>>(Array.Empty<Guid>());
        public Task<IReadOnlyList<PersonSourceId>> GetCurrentSourceIdsAsync(Guid tenantId, Guid personId, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<PersonSourceId>>(Array.Empty<PersonSourceId>());
        public Task<Guid?> ResolvePersonIdByAccountIdAsync(Guid tenantId, string accountId, CancellationToken cancellationToken)
            => Task.FromResult<Guid?>(null);
    }
}
