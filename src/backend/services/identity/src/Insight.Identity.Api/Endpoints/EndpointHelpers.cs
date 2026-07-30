using FluentValidation.Results;
using Insight.Identity.Api.Auth;
using Insight.Identity.Api.Contracts;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;

namespace Insight.Identity.Api.Endpoints;

/// <summary>
/// Shared endpoint utilities: caller/tenant resolution shortcuts,
/// RFC 7807 response shaping for the common gate / validation / not-
/// found paths, and a generic structured audit-log helper. Used by
/// every endpoint file that needs admin-gating or write-side audit.
/// </summary>
public static class EndpointHelpers
{
    public const string CallerUnresolvedUrn = "urn:insight:error:caller_unresolved";
    public const string TenantUnresolvedUrn = "urn:insight:error:tenant_unresolved";
    public const string AdminRequiredUrn    = "urn:insight:error:admin_required";
    public const string NotFoundUrn         = "urn:insight:error:not_found";

    public static Guid? ResolveTenant(HttpContext http) =>
        http.RequestServices.GetRequiredService<ITenantContext>().Resolve(http);

    public static Task<Guid?> ResolveCallerAsync(HttpContext http, CancellationToken cancellationToken) =>
        http.RequestServices.GetRequiredService<ICallerContext>().ResolveAsync(http, cancellationToken);

    public static IResult GateResult(AdminCheckResult gate) => gate switch
    {
        AdminCheckResult.NoCaller => Results.Json(new ProblemResponse(
            Type: CallerUnresolvedUrn,
            Title: "Unauthorized",
            Status: StatusCodes.Status401Unauthorized,
            Detail: "Caller not identified. The gateway JWT must carry a person subject (sub)."),
            statusCode: StatusCodes.Status401Unauthorized),
        AdminCheckResult.NoTenant => Results.Json(new ProblemResponse(
            Type: TenantUnresolvedUrn,
            Title: "Bad Request",
            Status: StatusCodes.Status400BadRequest,
            Detail: "Tenant not provided."),
            statusCode: StatusCodes.Status400BadRequest),
        AdminCheckResult.NotAdmin => Results.Json(new ProblemResponse(
            Type: AdminRequiredUrn,
            Title: "Forbidden",
            Status: StatusCodes.Status403Forbidden,
            Detail: "admin role required for this operation"),
            statusCode: StatusCodes.Status403Forbidden),
        _ => Results.Problem("unexpected gate result", statusCode: StatusCodes.Status500InternalServerError),
    };

    public static IResult NotFound(string resource, Guid id) =>
        Results.Json(new ProblemResponse(
            Type: NotFoundUrn,
            Title: "Not Found",
            Status: StatusCodes.Status404NotFound,
            Detail: $"{resource} with id '{id:D}' not found"),
            statusCode: StatusCodes.Status404NotFound);

    public static IResult ValidationFailure(ValidationResult validation)
    {
        var first = validation.Errors[0];
        return Results.Json(new ProblemResponse(
            Type: string.IsNullOrEmpty(first.ErrorCode) ? "urn:insight:error:invalid_request" : first.ErrorCode,
            Title: "Bad Request",
            Status: StatusCodes.Status400BadRequest,
            Detail: first.ErrorMessage),
            statusCode: StatusCodes.Status400BadRequest);
    }

#pragma warning disable CA1848, CA2254 // structured audit lines with caller-supplied fields; LoggerMessage.Define is overkill for the half-dozen actions and the template varies by call site by design
    public static void Audit(ILoggerFactory loggerFactory, string action, params (string Key, object? Value)[] fields)
    {
        var logger = loggerFactory.CreateLogger("Insight.Identity.Api.Audit");
        var template = "audit:{Action} " + string.Join(" ", fields.Select(f => $"{f.Key}={{{f.Key}}}"));
        var values = new object?[fields.Length + 1];
        values[0] = action;
        for (var i = 0; i < fields.Length; i++) values[i + 1] = fields[i].Value;
        logger.LogInformation(template, values);
    }
#pragma warning restore CA1848, CA2254
}
