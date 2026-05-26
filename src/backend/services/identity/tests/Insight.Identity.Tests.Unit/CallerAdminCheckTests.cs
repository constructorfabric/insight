using FluentAssertions;
using Insight.Identity.Api.Auth;
using Insight.Identity.Domain.Services;
using Microsoft.AspNetCore.Http;
using Xunit;

namespace Insight.Identity.Tests.Unit;

public sealed class CallerAdminCheckTests
{
    private static readonly Guid TenantId = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid CallerId = Guid.Parse("33333333-3333-3333-3333-333333333333");

    [Fact]
    public async Task Returns_NoCaller_when_caller_header_absent()
    {
        var sut = Build(callerId: null, tenantId: TenantId, hasAdmin: false);
        var result = await sut.CheckAsync(new DefaultHttpContext(), CancellationToken.None);
        result.Should().Be(AdminCheckResult.NoCaller);
    }

    [Fact]
    public async Task Returns_NoTenant_when_tenant_unresolved()
    {
        var sut = Build(callerId: CallerId, tenantId: null, hasAdmin: false);
        var result = await sut.CheckAsync(new DefaultHttpContext(), CancellationToken.None);
        result.Should().Be(AdminCheckResult.NoTenant);
    }

    [Fact]
    public async Task Returns_NotAdmin_when_caller_lacks_role()
    {
        var sut = Build(callerId: CallerId, tenantId: TenantId, hasAdmin: false);
        var result = await sut.CheckAsync(new DefaultHttpContext(), CancellationToken.None);
        result.Should().Be(AdminCheckResult.NotAdmin);
    }

    [Fact]
    public async Task Returns_IsAdmin_when_caller_holds_admin_role()
    {
        var sut = Build(callerId: CallerId, tenantId: TenantId, hasAdmin: true);
        var result = await sut.CheckAsync(new DefaultHttpContext(), CancellationToken.None);
        result.Should().Be(AdminCheckResult.IsAdmin);
    }

    [Fact]
    public async Task Asks_PersonRolesReader_for_Roles_Admin_in_resolved_tenant()
    {
        var probe = new RecordingReader(hasAdmin: true);
        var sut = new CallerAdminCheck(
            new StaticCallerContext(CallerId),
            new StaticTenantContext(TenantId),
            probe);

        var result = await sut.CheckAsync(new DefaultHttpContext(), CancellationToken.None);

        result.Should().Be(AdminCheckResult.IsAdmin);
        probe.LastTenant.Should().Be(TenantId);
        probe.LastPerson.Should().Be(CallerId);
        probe.LastRole.Should().Be(Roles.Admin);
    }

    private static CallerAdminCheck Build(Guid? callerId, Guid? tenantId, bool hasAdmin) =>
        new(
            new StaticCallerContext(callerId),
            new StaticTenantContext(tenantId),
            new RecordingReader(hasAdmin));

    private sealed class StaticCallerContext(Guid? id) : ICallerContext
    {
        public Task<Guid?> ResolveAsync(HttpContext context, CancellationToken cancellationToken) => Task.FromResult(id);
    }

    private sealed class StaticTenantContext(Guid? id) : ITenantContext
    {
        public Guid? Resolve(HttpContext context) => id;
    }

    private sealed class RecordingReader(bool hasAdmin) : IPersonRolesReader
    {
        public Guid LastTenant { get; private set; }
        public Guid LastPerson { get; private set; }
        public Guid LastRole { get; private set; }

        public Task<bool> HasActiveRoleAsync(Guid tenantId, Guid personId, Guid roleId, CancellationToken cancellationToken)
        {
            LastTenant = tenantId;
            LastPerson = personId;
            LastRole = roleId;
            return Task.FromResult(hasAdmin);
        }

        public Task<IReadOnlyList<PersonRole>> GetActiveByPersonAsync(Guid tenantId, Guid personId, CancellationToken cancellationToken)
            => throw new NotImplementedException("not exercised in this test");

        public Task<PersonRole?> GetPersonRoleByIdAsync(Guid tenantId, Guid personRoleId, CancellationToken cancellationToken)
            => throw new NotImplementedException("not exercised in this test");

        public Task<PagedResult<PersonRole>> ListAsync(Guid tenantId, Guid? filterByPerson, Guid? filterByRole, bool activeOnly, PageRequest page, CancellationToken cancellationToken)
            => throw new NotImplementedException("not exercised in this test");

        public Task<int> CountActiveByRoleAsync(Guid tenantId, Guid roleId, CancellationToken cancellationToken)
            => throw new NotImplementedException("not exercised in this test");
    }
}
