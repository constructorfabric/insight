using Insight.Identity.Api.Auth;
using Insight.Identity.Api.Configuration;
using Insight.Identity.Api.Contracts;
using Insight.Identity.Domain.Services;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Mvc;
using Microsoft.AspNetCore.Routing;
using Microsoft.Extensions.Options;

namespace Insight.Identity.Api.Endpoints;

/// <summary>
/// <para>
/// <c>GET /v1/subchart/{person_id}?depth=N&amp;valid_at=…</c> — depth-bounded
/// org subtree rooted at a person, gated by the same
/// <see cref="VisibilityService"/> that protects <c>/v1/persons</c> and
/// <c>POST /v1/profiles</c>. #348 Phase 3.
/// </para>
/// <para>
/// <c>GET /v1/subchart?depth=N&amp;valid_at=…</c> — forest variant (#344
/// follow-up): returns every root the caller can see (one tree per
/// visible top of the source's org chart), filtered by their visibility
/// grants. Singleton orphans are dropped from the response.
/// </para>
/// <para>
/// <c>valid_at</c> (optional, ISO 8601 / RFC 3339 datetime — UTC
/// recommended) — point-in-time lens (#582). Absent → current state.
/// Set → reflects org structure AND visibility grants as they looked
/// at that moment.
/// </para>
/// <para>
/// Accepted forms:
/// <list type="bullet">
///   <item><c>2026-05-17T10:30:45Z</c> — UTC, recommended.</item>
///   <item><c>2026-05-17T10:30:45.123Z</c> — UTC with fractional seconds.</item>
///   <item><c>2026-05-17T10:30:45</c> — no zone designator; treated as UTC
///   (normalised by the endpoint).</item>
///   <item><c>2026-05-17</c> — date-only; interpreted as midnight UTC
///   (<c>2026-05-17T00:00:00Z</c>).</item>
///   <item><c>2026-05-17T10:30:45-08:00</c> — negative offset; converted
///   to UTC.</item>
///   <item><c>2026-05-17T10:30:45%2B03:00</c> — positive offset; the
///   <c>+</c> must be URL-encoded as <c>%2B</c> because standard URL
///   parsers turn a raw <c>+</c> in a query string into a space.</item>
/// </list>
/// Future-dated values (more than one minute beyond UTC now) are rejected
/// with 400 <c>urn:insight:error:invalid_valid_at</c>.
/// </para>
/// </summary>
public static class SubchartEndpoints
{
    public static IEndpointRouteBuilder MapSubchartEndpoints(this IEndpointRouteBuilder app)
    {
        ArgumentNullException.ThrowIfNull(app);

        app.MapGet("/v1/subchart", async (
            HttpContext http,
            ITenantContext tenants,
            ICallerContext callers,
            SubchartService subchart,
            IOptions<AppOptions> options,
            int? depth,
            [FromQuery(Name = "valid_at")] DateTime? validAt,
            CancellationToken ct) =>
        {
            var tenantId = tenants.Resolve(http);
            if (tenantId is null)
            {
                return Results.Json(new ProblemResponse(
                    Type: "urn:insight:error:tenant_unresolved",
                    Title: "Bad Request",
                    Status: StatusCodes.Status400BadRequest,
                    Detail: "Tenant not resolved. The gateway JWT must carry a valid tenant_id claim."),
                    statusCode: StatusCodes.Status400BadRequest);
            }

            var callerPersonId = await callers.ResolveAsync(http, ct).ConfigureAwait(false);
            if (callerPersonId is null)
            {
                return Results.Json(new ProblemResponse(
                    Type: "urn:insight:error:caller_unresolved",
                    Title: "Unauthorized",
                    Status: StatusCodes.Status401Unauthorized,
                    Detail: "Caller not identified. The gateway JWT must carry a person subject (sub)."),
                    statusCode: StatusCodes.Status401Unauthorized);
            }

            if (depth is < 0)
            {
                return Results.Json(new ProblemResponse(
                    Type: "urn:insight:error:invalid_depth",
                    Title: "Bad Request",
                    Status: StatusCodes.Status400BadRequest,
                    Detail: $"depth must be >= 0; got {depth}"),
                    statusCode: StatusCodes.Status400BadRequest);
            }

            validAt = NormalizeValidAtToUtc(validAt);
            var validAtErr = ValidateValidAtNotFuture(validAt);
            if (validAtErr is not null) return validAtErr;

            var sourceType = options.Value.OrgChartSourceType;
            // Open to every authenticated caller — visibility decides
            // what the forest looks like. Empty visible set / empty
            // in-source membership → empty roots array, 200 not 404.
            var roots = await subchart
                .GetForestAsync(tenantId.Value, callerPersonId.Value, sourceType, depth, validAt, ct)
                .ConfigureAwait(false);
            return Results.Ok(new SubchartForestResponse(
                roots.Select(SubchartNodeResponse.From).ToList()));
        });

        app.MapGet("/v1/subchart/{personId:guid}", async (
            Guid personId,
            HttpContext http,
            ITenantContext tenants,
            ICallerContext callers,
            SubchartService subchart,
            IOptions<AppOptions> options,
            int? depth,
            [FromQuery(Name = "valid_at")] DateTime? validAt,
            CancellationToken ct) =>
        {
            var tenantId = tenants.Resolve(http);
            if (tenantId is null)
            {
                return Results.Json(new ProblemResponse(
                    Type: "urn:insight:error:tenant_unresolved",
                    Title: "Bad Request",
                    Status: StatusCodes.Status400BadRequest,
                    Detail: "Tenant not resolved. The gateway JWT must carry a valid tenant_id claim."),
                    statusCode: StatusCodes.Status400BadRequest);
            }

            var callerPersonId = await callers.ResolveAsync(http, ct).ConfigureAwait(false);
            if (callerPersonId is null)
            {
                return Results.Json(new ProblemResponse(
                    Type: "urn:insight:error:caller_unresolved",
                    Title: "Unauthorized",
                    Status: StatusCodes.Status401Unauthorized,
                    Detail: "Caller not identified. The gateway JWT must carry a person subject (sub)."),
                    statusCode: StatusCodes.Status401Unauthorized);
            }

            if (depth is < 0)
            {
                return Results.Json(new ProblemResponse(
                    Type: "urn:insight:error:invalid_depth",
                    Title: "Bad Request",
                    Status: StatusCodes.Status400BadRequest,
                    Detail: $"depth must be >= 0; got {depth}"),
                    statusCode: StatusCodes.Status400BadRequest);
            }

            validAt = NormalizeValidAtToUtc(validAt);
            var validAtErr = ValidateValidAtNotFuture(validAt);
            if (validAtErr is not null) return validAtErr;

            var sourceType = options.Value.OrgChartSourceType;
            // Per-node visibility filtering is deliberately omitted —
            // VisibilityService's CTE is closed under org_chart descent,
            // so once the caller can see the root, every descendant is
            // already in their visible set. Matches the Phase-2 behaviour
            // of `subordinates[]` on /v1/persons. If a per-person revoke
            // surface lands later, bulk per-node filtering would go here.
            var node = await subchart
                .GetSubchartAsync(tenantId.Value, callerPersonId.Value, personId, sourceType, depth, validAt, ct)
                .ConfigureAwait(false);
            if (node is null)
            {
                // Deny → 404 in the same shape as "not found" so existence
                // doesn't leak to a caller without visibility.
                return Results.Json(new ProblemResponse(
                    Type: "urn:insight:error:person_not_found",
                    Title: "Not Found",
                    Status: StatusCodes.Status404NotFound,
                    Detail: $"person {personId:D} not found or not visible"),
                    statusCode: StatusCodes.Status404NotFound);
            }
            return Results.Ok(new SubchartResponse(SubchartNodeResponse.From(node)));
        });

        return app;
    }

    /// <summary>
    /// Normalise <c>valid_at</c> to UTC after model binding (#582). The
    /// ASP.NET Core <see cref="DateTime"/> binder parses
    /// <c>?valid_at=2026-05-29T12:00:00Z</c> as <c>DateTimeKind.Utc</c>,
    /// values without a zone designator as <c>Unspecified</c>, and
    /// offset-form values as <c>Local</c>. The endpoint contract is
    /// UTC, so we coerce: <c>Unspecified</c> is treated as already-UTC
    /// (matches the contract), <c>Local</c> is converted via
    /// <see cref="DateTime.ToUniversalTime"/>.
    /// </summary>
    private static DateTime? NormalizeValidAtToUtc(DateTime? validAt) => validAt switch
    {
        null                                       => null,
        { Kind: DateTimeKind.Utc }                 => validAt,
        { Kind: DateTimeKind.Local }       v       => v.ToUniversalTime(),
        { Kind: DateTimeKind.Unspecified } v       => DateTime.SpecifyKind(v, DateTimeKind.Utc),
        _                                          => validAt,
    };

    /// <summary>
    /// Reject future-dated <c>valid_at</c> with 400 (#582). Allows a
    /// one-minute slack for clock skew between the caller and the API.
    /// Both subchart endpoints route through this helper so the URN
    /// and detail format cannot drift between handlers.
    /// </summary>
    private static IResult? ValidateValidAtNotFuture(DateTime? validAt)
    {
        if (validAt is { } ts && ts > DateTime.UtcNow.AddMinutes(1))
        {
            return Results.Json(new ProblemResponse(
                Type: "urn:insight:error:invalid_valid_at",
                Title: "Bad Request",
                Status: StatusCodes.Status400BadRequest,
                Detail: $"valid_at must not be in the future; got {ts:O}"),
                statusCode: StatusCodes.Status400BadRequest);
        }
        return null;
    }
}
