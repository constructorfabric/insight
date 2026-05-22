using Insight.Identity.Api;
using Insight.Identity.Api.Auth;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.Extensions.Configuration;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// Boots the API in-process against the Testcontainers MariaDB. The
/// fixture's connection string is fed in via <c>mariadb:connection_string</c>
/// so the API uses the exact MySqlConnector settings the container
/// negotiated (<c>SslMode</c>, <c>AllowPublicKeyRetrieval</c>, etc.) —
/// going through the URL form would lose those parameters and trip
/// connection timeouts in some Docker configurations. Pass a
/// <c>defaultTenantId</c> to wire the config-default resolver, or
/// <c>null</c> to exercise the missing-tenant 400 branch. Pass a
/// <c>defaultCallerPersonId</c> so every client request carries the
/// <c>X-Insight-Person-Id</c> header — necessary now that
/// <c>/v1/persons</c> + <c>POST /v1/profiles</c> 401 without a caller.
/// </summary>
public sealed class TestApplicationFactory : WebApplicationFactory<Program>
{
    private readonly string _databaseConnectionString;
    private readonly Guid? _defaultTenantId;
    private readonly Guid? _defaultCallerPersonId;
    private readonly bool? _expandSubordinates;

    public TestApplicationFactory(
        string databaseConnectionString,
        Guid? defaultTenantId,
        Guid? defaultCallerPersonId = null,
        bool? expandSubordinates = null)
    {
        _databaseConnectionString = databaseConnectionString;
        _defaultTenantId = defaultTenantId;
        _defaultCallerPersonId = defaultCallerPersonId;
        _expandSubordinates = expandSubordinates;
    }

    protected override void ConfigureWebHost(IWebHostBuilder builder)
    {
        builder.UseSetting("ContentRoot", AppContext.BaseDirectory);

        builder.ConfigureAppConfiguration((_, config) =>
        {
            var dict = new Dictionary<string, string?>
            {
                ["mariadb:connection_string"] = _databaseConnectionString,
                // Defensively zero out timeout/pool knobs in case a
                // future appsettings.yaml change re-introduces small
                // values that would suffocate the MariaDB handshake.
                ["mariadb:url"] = "",
                ["mariadb:connection_timeout_seconds"] = "0",
                ["mariadb:command_timeout_seconds"] = "0",
                ["mariadb:min_pool_size"] = "0",
                ["mariadb:max_pool_size"] = "0",
                ["identity:bind_addr"] = "0.0.0.0:0",
            };
            if (_defaultTenantId is { } tenant)
            {
                dict["identity:tenant_default_id"] = tenant.ToString("D");
            }
            if (_expandSubordinates is { } expand)
            {
                dict["identity:expand_subordinates"] = expand ? "true" : "false";
            }
            config.AddInMemoryCollection(dict);
        });
    }

    protected override void ConfigureClient(System.Net.Http.HttpClient client)
    {
        base.ConfigureClient(client);
        if (_defaultCallerPersonId is { } caller)
        {
            client.DefaultRequestHeaders.Add(HeaderCallerContext.HeaderName, caller.ToString("D"));
        }
    }
}
