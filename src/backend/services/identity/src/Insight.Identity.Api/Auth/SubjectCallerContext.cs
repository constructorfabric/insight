using Microsoft.AspNetCore.Http;

namespace Insight.Identity.Api.Auth;

/// <summary>
/// Resolves the caller's <c>person_id</c> straight from the gateway JWT's
/// signed <c>sub</c> claim (NGINX_BFF step 07). Under the gateway-JWT contract
/// <c>sub</c> <b>is</b> the internal person id, so there is no IdP-account /
/// email lookup and no <c>X-Insight-Person-Id</c> header trust path — those
/// belonged to the raw-customer-IdP world this step removes.
///
/// Returns <c>null</c> when <c>sub</c> is absent or not a person UUID — e.g. a
/// service token whose <c>sub</c> is <c>service:&lt;name&gt;</c>. Such callers
/// carry no person; endpoints that require an identified person respond 401/403
/// accordingly (they authorize service work on the <c>service</c> role instead).
/// </summary>
public sealed class SubjectCallerContext : ICallerContext
{
    /// <summary><c>sub</c> prefix that marks a service token, not a person.</summary>
    public const string ServiceSubjectPrefix = "service:";

    public Task<Guid?> ResolveAsync(HttpContext context, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(context);

        var sub = context.User.FindFirst("sub")?.Value;
        if (string.IsNullOrWhiteSpace(sub) || sub.StartsWith(ServiceSubjectPrefix, StringComparison.Ordinal))
        {
            return Task.FromResult<Guid?>(null);
        }

        return Task.FromResult(
            Guid.TryParse(sub, out var personId) && personId != Guid.Empty ? (Guid?)personId : null);
    }
}
