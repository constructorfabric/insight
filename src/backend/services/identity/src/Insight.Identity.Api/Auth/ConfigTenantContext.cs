using Insight.Identity.Api.Configuration;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.Options;

namespace Insight.Identity.Api.Auth;

/// <summary>
/// Falls back to the <c>identity.tenant_default_id</c> setting (env
/// <c>IDENTITY__tenant_default_id</c>). Useful for single-tenant
/// development clusters and the local gitops environment.
/// </summary>
public sealed class ConfigTenantContext : ITenantContext
{
    private readonly Guid? _default;

    public ConfigTenantContext(IOptions<AppOptions> options)
    {
        ArgumentNullException.ThrowIfNull(options);
        _default = options.Value.TenantDefaultId;
    }

    public Guid? Resolve(HttpContext context) => _default;
}
