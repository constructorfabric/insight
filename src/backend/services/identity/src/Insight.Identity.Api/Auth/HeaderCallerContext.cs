using Insight.Identity.Domain.Services;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.Logging;

namespace Insight.Identity.Api.Auth;

/// <summary>
/// Resolves the caller's <c>person_id</c>. Tries the
/// <c>X-Insight-Person-Id</c> header first; then JWT id claims
/// (<c>oid</c> / <c>sub</c>) via <c>account_person_map</c>; then JWT
/// email claims (<c>email</c> / <c>preferred_username</c> / <c>upn</c>)
/// via <c>persons.value_type='email'</c>. Result is cached on
/// <see cref="HttpContext.Items"/> for the request. The DB lookups
/// filter by tenant — endpoints are expected to gate on tenant
/// (400 <c>tenant_unresolved</c>) before reading the caller, so the
/// resolver only runs when a tenant is in scope.
/// </summary>
public sealed class HeaderCallerContext : ICallerContext
{
    public const string HeaderName = "X-Insight-Person-Id";
    private const string CacheKey = "__insight_caller_person_id__";

    private static readonly string[] IdClaims    = ["oid", "sub"];
    private static readonly string[] EmailClaims = ["email", "preferred_username", "upn"];

    private readonly IPersonsReader _reader;
    private readonly ITenantContext _tenant;
    private readonly ILogger<HeaderCallerContext> _log;

    public HeaderCallerContext(
        IPersonsReader reader,
        ITenantContext tenant,
        ILogger<HeaderCallerContext> log)
    {
        _reader = reader;
        _tenant = tenant;
        _log = log;
    }

    public async Task<Guid?> ResolveAsync(HttpContext context, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(context);

        // Cache the result (including null) so a single request does
        // not hit MariaDB more than once.
        if (context.Items.TryGetValue(CacheKey, out var cached))
        {
            return (Guid?)cached;
        }
        var resolved = await ResolveInternalAsync(context, cancellationToken).ConfigureAwait(false);
        context.Items[CacheKey] = resolved;
        return resolved;
    }

    private async Task<Guid?> ResolveInternalAsync(HttpContext context, CancellationToken cancellationToken)
    {
        // 1. Header wins when present. The header path needs no tenant
        //    — lets a future api-gateway BFF pre-resolve once and skip
        //    the JWT steps below.
        if (context.Request.Headers.TryGetValue(HeaderName, out var raw)
            && Guid.TryParse(raw.ToString(), out var headerPersonId)
            && headerPersonId != Guid.Empty)
        {
            return headerPersonId;
        }

        // JWT-driven resolution needs the tenant — every read on
        // account_person_map / persons is tenant-scoped.
        var tenantId = _tenant.Resolve(context);
        if (tenantId is null)
        {
            return null;
        }

        // 2. JWT id claims (oid / sub) → account_person_map.
        foreach (var claim in IdClaims)
        {
            var value = context.User.FindFirst(claim)?.Value?.Trim();
            if (string.IsNullOrEmpty(value))
            {
                continue;
            }
            var matched = await _reader
                .ResolvePersonIdByAccountIdAsync(tenantId.Value, value, cancellationToken)
                .ConfigureAwait(false);
            if (matched is not null && matched != Guid.Empty)
            {
                return matched;
            }
        }

        // 3. JWT email claims → persons.value_type='email'.
        //    TODO(#346-follow-up): Count > 1 means two persons in the
        //    same tenant share an email — corrupted state. We skip
        //    rather than pick one at random, which surfaces as 401.
        //    Will need a clearer error path when multi-tenant rolls in.
        foreach (var claim in EmailClaims)
        {
            var value = context.User.FindFirst(claim)?.Value?.Trim();
            if (string.IsNullOrEmpty(value))
            {
                continue;
            }
            var matches = await _reader
                .ResolvePersonIdsByEmailAsync(tenantId.Value, value, cancellationToken)
                .ConfigureAwait(false);
            if (matches.Count == 1 && matches[0] != Guid.Empty)
            {
                return matches[0];
            }
            if (matches.Count > 1)
            {
#pragma warning disable CA1848 // structured warning on a rare path; LoggerMessage.Define is overkill
                _log.LogWarning(
                    "Ambiguous JWT caller: claim {Claim}={Value} matches {Count} persons in tenant {Tenant}",
                    claim, value, matches.Count, tenantId);
#pragma warning restore CA1848
            }
        }

        return null;
    }
}
