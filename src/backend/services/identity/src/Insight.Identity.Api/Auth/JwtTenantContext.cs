using Microsoft.AspNetCore.Http;

namespace Insight.Identity.Api.Auth;

/// <summary>
/// Tenant resolver that reads the <c>insight_tenant_id</c> claim from
/// the bearer token decoded by the JwtBearer middleware (see
/// <c>Program.cs</c> auth wiring). Returns <c>null</c> when no claim is
/// present so the composite chain can fall through to the next
/// resolver. Validation of the token itself is owned by api-gateway;
/// this service runs parse-only until #346 pins per-env IdP config.
/// </summary>
public sealed class JwtTenantContext : ITenantContext
{
    public Guid? Resolve(HttpContext context)
    {
        ArgumentNullException.ThrowIfNull(context);
        var raw = context.User.FindFirst("insight_tenant_id")?.Value;
        return Guid.TryParse(raw, out var tenantId) ? tenantId : null;
    }
}
