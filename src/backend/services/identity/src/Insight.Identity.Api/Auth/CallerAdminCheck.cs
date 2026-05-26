using Insight.Identity.Domain.Services;
using Microsoft.AspNetCore.Http;

namespace Insight.Identity.Api.Auth;

/// <summary>
/// Single-call admin probe for CRUD endpoints: resolves the caller +
/// tenant from the request, then asks <see cref="IPersonRolesReader"/>
/// whether the caller currently holds the <c>admin</c> role in that
/// tenant. Returns <c>false</c> when either resolver fails — callers
/// then map to 401 (no caller) / 400 (no tenant) / 403 (not admin)
/// per their endpoint contract.
/// </summary>
public sealed class CallerAdminCheck
{
    private readonly ICallerContext _caller;
    private readonly ITenantContext _tenant;
    private readonly IPersonRolesReader _personRoles;

    public CallerAdminCheck(ICallerContext caller, ITenantContext tenant, IPersonRolesReader personRoles)
    {
        _caller = caller;
        _tenant = tenant;
        _personRoles = personRoles;
    }

    public async Task<AdminCheckResult> CheckAsync(HttpContext context, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(context);
        // Tenant first so the caller resolver (which itself needs the
        // tenant for its DB lookups) only runs when tenant is in scope.
        var tenantId = _tenant.Resolve(context);
        if (tenantId is null)
        {
            return AdminCheckResult.NoTenant;
        }
        var personId = await _caller.ResolveAsync(context, cancellationToken).ConfigureAwait(false);
        if (personId is null)
        {
            return AdminCheckResult.NoCaller;
        }
        var hasAdmin = await _personRoles
            .HasActiveRoleAsync(tenantId.Value, personId.Value, Roles.Admin, cancellationToken)
            .ConfigureAwait(false);
        return hasAdmin ? AdminCheckResult.IsAdmin : AdminCheckResult.NotAdmin;
    }
}

/// <summary>Outcome of <see cref="CallerAdminCheck.CheckAsync"/>.</summary>
public enum AdminCheckResult
{
    /// <summary>No caller header → endpoint should respond 401.</summary>
    NoCaller,
    /// <summary>No tenant resolved → endpoint should respond 400.</summary>
    NoTenant,
    /// <summary>Caller resolved but lacks <c>admin</c> role → endpoint should respond 403.</summary>
    NotAdmin,
    /// <summary>Caller holds <c>admin</c> in the tenant → endpoint may proceed.</summary>
    IsAdmin,
}
