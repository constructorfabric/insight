using Microsoft.Extensions.Configuration;

namespace Insight.Identity.Api.Configuration;

/// <summary>
/// Top-level service options bound from the <c>identity</c>
/// configuration section. The <c>IDENTITY__*</c> double-underscore
/// env-var layout is normalized by the configuration providers in
/// <c>Program.cs</c>; <see cref="ConfigurationKeyNameAttribute"/>
/// bridges the snake_case keys to PascalCase properties because the
/// default binder only does case-insensitive matching, not separator
/// translation.
/// </summary>
public sealed class AppOptions
{
    public const string SectionName = "identity";

    /// <summary>HTTP listener bind address.</summary>
    [ConfigurationKeyName("bind_addr")]
    public string BindAddr { get; init; } = "0.0.0.0:8082";

    /// <summary>
    /// Single-tenant fallback used only when the gateway JWT carries no
    /// <c>tenants</c> claim (e.g. cross-tenant service tokens) — never as a
    /// way to override the signed set.
    /// </summary>
    [ConfigurationKeyName("tenant_default_id")]
    public Guid? TenantDefaultId { get; init; }

    /// <summary>
    /// Expected issuer (<c>iss</c>) of the gateway JWT — the authenticator's
    /// gateway origin. Verified fail-closed (NGINX_BFF R1). Env:
    /// <c>IDENTITY__auth_gateway_issuer</c>.
    /// </summary>
    [ConfigurationKeyName("auth_gateway_issuer")]
    public string AuthGatewayIssuer { get; init; } = "";

    /// <summary>
    /// JWKS endpoint the gateway JWT's signing keys are fetched from (the
    /// gateway's / authenticator's <c>/.well-known/jwks.json</c>). This is the
    /// per-service <c>GATEWAY_JWKS_URL</c>. Env:
    /// <c>IDENTITY__auth_gateway_jwks_url</c>.
    /// </summary>
    [ConfigurationKeyName("auth_gateway_jwks_url")]
    public string AuthGatewayJwksUrl { get; init; } = "";

    /// <summary>
    /// Kill switch for the recursive org-tree walk on
    /// <c>/v1/persons</c> and <c>/v1/profiles</c>.
    /// </summary>
    [ConfigurationKeyName("expand_subordinates")]
    public bool ExpandSubordinates { get; init; } = true;

    /// <summary>Hard cap on org-tree recursion depth.</summary>
    [ConfigurationKeyName("max_subordinate_depth")]
    public int MaxSubordinateDepth { get; init; } = 16;

    /// <summary>
    /// Which <c>insight_source_type</c> drives the org-tree
    /// projection (parent + subordinates) returned by the lookup
    /// endpoints. Other sources still contribute to attribute
    /// hydration and the <c>ids[]</c> list but stay invisible to the
    /// tree.
    /// </summary>
    [ConfigurationKeyName("org_chart_source_type")]
    public string OrgChartSourceType { get; init; } = "bamboohr";

    /// <summary>
    /// First admin (<c>person_id</c>) to seed into <c>person_roles</c>
    /// on startup so the CRUD endpoints have at least one caller who
    /// can mint further grants. <c>null</c> = skip the bootstrap step.
    /// Idempotent: only inserts when no active assignment exists for
    /// <c>(tenant_default_id, this person, Roles.Admin)</c>.
    /// </summary>
    [ConfigurationKeyName("bootstrap_admin_person_id")]
    public Guid? BootstrapAdminPersonId { get; init; }
}
