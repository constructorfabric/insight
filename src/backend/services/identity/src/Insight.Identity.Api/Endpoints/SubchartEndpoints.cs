using Insight.Identity.Api.Auth;
using Insight.Identity.Api.Configuration;
using Insight.Identity.Api.Contracts;
using Insight.Identity.Domain.Services;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Routing;
using Microsoft.Extensions.Options;

namespace Insight.Identity.Api.Endpoints;

/// <summary>
/// <c>GET /v1/subchart/{person_id}?depth=N</c> — depth-bounded org
/// subtree rooted at a person, gated by the same
/// <see cref="VisibilityService"/> that protects <c>/v1/persons</c> and
/// <c>POST /v1/profiles</c>. #348 Phase 3.
/// </summary>
public static class SubchartEndpoints
{
    public static IEndpointRouteBuilder MapSubchartEndpoints(this IEndpointRouteBuilder app)
    {
        ArgumentNullException.ThrowIfNull(app);

        app.MapGet("/v1/subchart/{personId:guid}", async (
            Guid personId,
            HttpContext http,
            ITenantContext tenants,
            ICallerContext callers,
            SubchartService subchart,
            IOptions<AppOptions> options,
            int? depth,
            CancellationToken ct) =>
        {
            var tenantId = tenants.Resolve(http);
            if (tenantId is null)
            {
                return Results.Json(new ProblemResponse(
                    Type: "urn:insight:error:tenant_unresolved",
                    Title: "Bad Request",
                    Status: StatusCodes.Status400BadRequest,
                    Detail: $"Tenant not provided. Send the {HeaderTenantContext.HeaderName} header or configure identity.tenant_default_id."),
                    statusCode: StatusCodes.Status400BadRequest);
            }

            var callerPersonId = await callers.ResolveAsync(http, ct).ConfigureAwait(false);
            if (callerPersonId is null)
            {
                return Results.Json(new ProblemResponse(
                    Type: "urn:insight:error:caller_unresolved",
                    Title: "Unauthorized",
                    Status: StatusCodes.Status401Unauthorized,
                    Detail: $"Caller not identified. Send the {HeaderCallerContext.HeaderName} header."),
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

            var sourceType = options.Value.OrgChartSourceType;
            // Per-node visibility filtering is deliberately omitted —
            // VisibilityService's CTE is closed under org_chart descent,
            // so once the caller can see the root, every descendant is
            // already in their visible set. Matches the Phase-2 behaviour
            // of `subordinates[]` on /v1/persons. If a per-person revoke
            // surface lands later, bulk per-node filtering would go here.
            var node = await subchart
                .GetSubchartAsync(tenantId.Value, callerPersonId.Value, personId, sourceType, depth, ct)
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
}
