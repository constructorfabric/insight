using System.Globalization;
using Microsoft.AspNetCore.Http;

namespace Insight.Identity.Api.Auth;

/// <summary>
/// Tenant resolver for the gateway JWT (NGINX_BFF §10 G2). The single signed
/// <c>tenant_id</c> claim is the <b>only</b> tenant authority: a token carries
/// exactly one tenant, minted by the authenticator from the session. There is
/// no <c>X-Tenant-ID</c> selector and no multi-tenant <c>tenants[]</c> array —
/// a tenant from the outside world never passes.
///
/// Returns the parsed tenant UUID, or <c>null</c> when the token carries no
/// (valid) <c>tenant_id</c> claim — there is no default fallback, so a request
/// without a signed tenant fails closed (400 tenant_unresolved) rather than
/// reading another tenant's data. The gateway JWT's signature/issuer/audience
/// are already verified by the JwtBearer pipeline before this runs.
/// </summary>
public sealed class GatewayTenantContext : ITenantContext
{
    /// <summary>The single signed tenant authority claim (a tenant UUID).</summary>
    public const string TenantClaim = "tenant_id";

    public Guid? Resolve(HttpContext context)
    {
        ArgumentNullException.ThrowIfNull(context);

        var raw = context.User.FindFirst(TenantClaim)?.Value;
        if (string.IsNullOrWhiteSpace(raw))
        {
            return null;
        }
        return Guid.TryParse(raw.Trim(), CultureInfo.InvariantCulture, out var id) && id != Guid.Empty
            ? id
            : null;
    }
}
