using FluentAssertions;
using Insight.Identity.Domain.Services;
using Insight.Identity.Infrastructure.MariaDb;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;
using MySqlConnector;
using Xunit;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// Verifies that <see cref="BootstrapAdminRunner"/> seeds the first
/// admin idempotently — second run is a no-op, missing tenant skips.
/// </summary>
[Collection(MariaDbCollection.Name)]
public sealed class BootstrapAdminTests : IAsyncLifetime
{
    private static readonly Guid TenantId       = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid AdminPersonId  = Guid.Parse("00000000-0000-0000-0000-000000000bbb");

    private readonly MariaDbFixture _fixture;
    private MariaDbConnectionFactory? _factory;
    private RolesRepository? _roles;

    public BootstrapAdminTests(MariaDbFixture fixture) => _fixture = fixture;

    public async Task InitializeAsync()
    {
        await _fixture.ResetAsync().ConfigureAwait(false);
        _factory = new MariaDbConnectionFactory(
            new OptionsWrapper<MariaDbOptions>(new MariaDbOptions { ConnectionString = _fixture.ConnectionString }));
        _roles = new RolesRepository(_factory);
    }

    public Task DisposeAsync() => Task.CompletedTask;

    [Fact]
    public async Task First_run_inserts_the_admin_assignment()
    {
        await BootstrapAdminRunner.RunAsync(_factory!, TenantId, AdminPersonId, NullLogger.Instance).ConfigureAwait(false);

        var hasAdmin = await _roles!.HasActiveRoleAsync(TenantId, AdminPersonId, Roles.Admin, CancellationToken.None);
        hasAdmin.Should().BeTrue();
    }

    [Fact]
    public async Task Second_run_does_not_create_a_duplicate()
    {
        await BootstrapAdminRunner.RunAsync(_factory!, TenantId, AdminPersonId, NullLogger.Instance).ConfigureAwait(false);
        await BootstrapAdminRunner.RunAsync(_factory!, TenantId, AdminPersonId, NullLogger.Instance).ConfigureAwait(false);

        var active = await _roles!.GetActiveByPersonAsync(TenantId, AdminPersonId, CancellationToken.None);
        active.Where(a => a.RoleId == Roles.Admin).Should().ContainSingle();
    }

    [Fact]
    public async Task Run_with_null_admin_id_is_a_noop()
    {
        await BootstrapAdminRunner.RunAsync(_factory!, TenantId, null, NullLogger.Instance).ConfigureAwait(false);

        var hasAdmin = await _roles!.HasActiveRoleAsync(TenantId, AdminPersonId, Roles.Admin, CancellationToken.None);
        hasAdmin.Should().BeFalse();
    }

    [Fact]
    public async Task Run_with_null_tenant_is_a_noop()
    {
        await BootstrapAdminRunner.RunAsync(_factory!, null, AdminPersonId, NullLogger.Instance).ConfigureAwait(false);

        // No tenant means we can't even check the right row; verify nothing
        // got inserted by counting active rows directly.
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        await using var cmd = new MySqlCommand(
            "SELECT COUNT(*) FROM person_roles WHERE person_id = @p AND valid_to IS NULL", conn);
        cmd.Parameters.AddWithValue("@p", AdminPersonId.ToByteArray(bigEndian: true));
        var count = (long)(await cmd.ExecuteScalarAsync().ConfigureAwait(false))!;
        count.Should().Be(0);
    }
}
