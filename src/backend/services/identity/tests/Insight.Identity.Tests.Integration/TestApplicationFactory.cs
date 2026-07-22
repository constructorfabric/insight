using Insight.Identity.Api;
using Microsoft.AspNetCore.Authentication.JwtBearer;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.AspNetCore.TestHost;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.IdentityModel.JsonWebTokens;
using Microsoft.IdentityModel.Tokens;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// Boots the API in-process against the Testcontainers MariaDB.
///
/// Auth: the service verifies a gateway JWT fail-closed (NGINX_BFF R1). Rather
/// than stand up a JWKS server per test, the factory loosens the JwtBearer
/// validation for the test host (signature/issuer/audience checks off) so a
/// hand-built token with the right claims authenticates — the real ES256/JWKS
/// path is proven by the compose e2e. A <c>defaultCallerPersonId</c> attaches a
/// default bearer whose <c>sub</c> is that person (the gateway JWT's <c>sub</c>
/// IS the internal person_id) and whose <c>tenant_id</c> claim carries the
/// tenant (the only tenant authority — there is no config-default fallback). Pass
/// <c>null</c> callers / tenants to exercise the missing-caller / missing-tenant
/// branches.
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

        // The fail-closed gateway-JWT wiring requires these two settings, and
        // Program.cs reads them BEFORE `builder.Build()` — so a
        // ConfigureAppConfiguration source (applied only at Build) is too late,
        // and appsettings.yaml declares them empty (overriding host settings).
        // The `IDENTITY__`-prefixed env source is added last in Program.cs and
        // is read at CreateBuilder time, so it is the one lever that is both
        // visible pre-Build and wins over the empty appsettings default. Values
        // are dummy `.invalid`; JwtBearer validation is loosened below so they
        // are never fetched.
        Environment.SetEnvironmentVariable(
            "IDENTITY__identity__auth_gateway_issuer", "https://gateway.test.invalid");
        Environment.SetEnvironmentVariable(
            "IDENTITY__identity__auth_gateway_jwks_url",
            "https://gateway.test.invalid/.well-known/jwks.json");
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

        // Test host only: accept a hand-built (unsigned) gateway JWT so tests
        // authenticate without a live JWKS. Production keeps the strict ES256 /
        // JWKS validation from Program.cs.
        builder.ConfigureTestServices(services =>
        {
            services.PostConfigure<JwtBearerOptions>(
                JwtBearerDefaults.AuthenticationScheme,
                options =>
                {
                    options.TokenValidationParameters = new TokenValidationParameters
                    {
                        ValidateIssuer = false,
                        ValidateAudience = false,
                        ValidateLifetime = false,
                        ValidateIssuerSigningKey = false,
                        RequireSignedTokens = false,
                        SignatureValidator = (token, _) => new JsonWebToken(token),
                    };
                });
        });
    }

    protected override void ConfigureClient(System.Net.Http.HttpClient client)
    {
        base.ConfigureClient(client);
        // Always attach a bearer so the request clears the fail-closed
        // `RequireAuthenticatedUser` fallback policy (NGINX_BFF R1) and reaches
        // the endpoint. A null caller means "authenticated but no resolvable
        // caller": a token WITHOUT a `sub`, so the endpoint's caller resolution
        // returns the `caller_unresolved` 401 (the true token-less middleware
        // 401 is covered by the compose e2e, not in-process here).
        // Tenant is the signed `tenant_id` claim (GatewayTenantContext) — no more
        // config-default fallback. A null tenant omits the claim, so those tests
        // exercise the fail-closed missing-tenant (400) branch.
        var claims = new System.Collections.Generic.List<(string, string)>();
        if (_defaultCallerPersonId is { } caller)
        {
            claims.Add(("sub", caller.ToString("D")));
        }
        if (_defaultTenantId is { } tenant)
        {
            claims.Add(("tenant_id", tenant.ToString("D")));
        }
        var token = BuildJwt(claims.ToArray());
        client.DefaultRequestHeaders.Authorization =
            new System.Net.Http.Headers.AuthenticationHeaderValue("Bearer", token);
    }

    /// <summary>
    /// Build a hand-shaped gateway JWT with the given string claims. The test
    /// host does not verify the signature (see <see cref="ConfigureWebHost"/>),
    /// so a placeholder signature is fine.
    /// </summary>
    public static string BuildJwt(params (string Name, string Value)[] claims)
    {
        static string B64Url(string raw)
        {
            var bytes = System.Text.Encoding.UTF8.GetBytes(raw);
            return Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');
        }
        var header = B64Url("{\"alg\":\"ES256\",\"typ\":\"JWT\"}");
        var payloadJson = "{" + string.Join(",", claims.Select(c => $"\"{c.Name}\":\"{c.Value}\"")) + "}";
        var payload = B64Url(payloadJson);
        return $"{header}.{payload}.AAAA";
    }
}
