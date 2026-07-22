using System.Security.Claims;
using FluentAssertions;
using Insight.Identity.Api.Auth;
using Microsoft.AspNetCore.Http;
using Xunit;

namespace Insight.Identity.Tests.Unit;

public sealed class GatewayTenantContextTests
{
    private static readonly Guid TenantA = Guid.Parse("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");

    private static DefaultHttpContext Context(string? tenantId)
    {
        var claims = tenantId is null
            ? Array.Empty<Claim>()
            : [new Claim(GatewayTenantContext.TenantClaim, tenantId)];
        return new DefaultHttpContext
        {
            User = new ClaimsPrincipal(new ClaimsIdentity(claims, "test")),
        };
    }

    [Fact]
    public void Signed_tenant_id_resolves_it()
    {
        var result = new GatewayTenantContext().Resolve(Context(TenantA.ToString()));
        result.Should().Be(TenantA);
    }

    [Fact]
    public void No_tenant_claim_falls_through()
    {
        // The ONLY fall-through: no signed tenant → null so the config default
        // (bootstrap/admin context) can apply.
        var result = new GatewayTenantContext().Resolve(Context(tenantId: null));
        result.Should().BeNull();
    }

    [Fact]
    public void Blank_tenant_claim_falls_through()
    {
        new GatewayTenantContext().Resolve(Context("   ")).Should().BeNull();
    }

    [Fact]
    public void Malformed_tenant_claim_falls_through()
    {
        // A non-UUID (or empty-UUID) value never resolves a tenant — it can
        // never introduce one, so it degrades to the fall-through, not a grant.
        new GatewayTenantContext().Resolve(Context("not-a-uuid")).Should().BeNull();
        new GatewayTenantContext().Resolve(Context(Guid.Empty.ToString())).Should().BeNull();
    }
}
