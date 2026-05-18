using FluentValidation;
using Insight.Identity.Api.Auth;
using Insight.Identity.Api.Configuration;
using Insight.Identity.Api.Contracts;
using Insight.Identity.Domain.Services;
using Insight.Identity.Infrastructure.MariaDb;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Routing;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;

namespace Insight.Identity.Api.Endpoints;

public static class PersonsEndpoints
{
    public static IEndpointRouteBuilder MapPersonsEndpoints(this IEndpointRouteBuilder app)
    {
        ArgumentNullException.ThrowIfNull(app);

        app.MapGet("/v1/persons/{email}", async (
            string email,
            HttpContext http,
            ITenantContext tenants,
            PersonLookupService lookup,
            IOptions<AppOptions> options,
            CancellationToken cancellationToken) =>
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

            var lookupOptions = options.Value.ExpandSubordinates
                ? new LookupOptions(ExpandSubordinates: true, MaxDepth: options.Value.MaxSubordinateDepth)
                : LookupOptions.Phase1;

            var person = await lookup.GetByEmailAsync(tenantId.Value, email, lookupOptions, cancellationToken)
                .ConfigureAwait(false);
            if (person is null)
            {
                return Results.Json(new ProblemResponse(
                    Type: "urn:insight:error:person_not_found",
                    Title: "Not Found",
                    Status: StatusCodes.Status404NotFound,
                    Detail: $"person with email '{email}' not found"),
                    statusCode: StatusCodes.Status404NotFound);
            }

            return Results.Ok(PersonResponse.From(person));
        });

        app.MapPost("/v1/profiles", async (
            ResolveProfileCommandModel body,
            HttpContext http,
            ITenantContext tenants,
            ProfileLookupService lookup,
            IValidator<ResolveProfileCommandModel> validator,
            CancellationToken cancellationToken) =>
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

            var validation = await validator.ValidateAsync(body, cancellationToken).ConfigureAwait(false);
            if (!validation.IsValid)
            {
                // First-error-wins for the URN to keep the response shape
                // simple; client gets one urn:insight:error:* per call.
                var first = validation.Errors[0];
                return Results.Json(new ProblemResponse(
                    Type: string.IsNullOrEmpty(first.ErrorCode) ? "urn:insight:error:invalid_request" : first.ErrorCode,
                    Title: "Bad Request",
                    Status: StatusCodes.Status400BadRequest,
                    Detail: first.ErrorMessage),
                    statusCode: StatusCodes.Status400BadRequest);
            }

            var kind = body.ValueType == "id" ? ResolveProfileKind.SourceId : ResolveProfileKind.Email;
            var query = new ResolveProfileQuery(
                Kind: kind,
                Value: body.Value!,
                SourceType: body.InsightSourceType,
                SourceId: body.InsightSourceId);

            var result = await lookup.ResolveAsync(tenantId.Value, query, cancellationToken).ConfigureAwait(false);
            return result switch
            {
                ProfileLookupResult.Found f => Results.Ok(ProfileResponse.From(f.Profile)),
                ProfileLookupResult.NotFound => Results.Json(new ProblemResponse(
                        Type: "urn:insight:error:person_not_found",
                        Title: "Not Found",
                        Status: StatusCodes.Status404NotFound,
                        Detail: body.ValueType == "email"
                            ? $"no current observation matches email '{body.Value}' for the tenant"
                            : $"no current observation matches value_type='id' value='{body.Value}' within the given source instance"),
                    statusCode: StatusCodes.Status404NotFound),
                ProfileLookupResult.Ambiguous a => Results.Json(new AmbiguousProfileProblemResponse(
                        Type: "urn:insight:error:ambiguous_profile",
                        Title: "Data Invariant Violated",
                        Status: StatusCodes.Status422UnprocessableEntity,
                        Detail: $"lookup matched {a.PersonIds.Count} distinct person_ids; invariant requires exactly 1",
                        Lookup: body,
                        PersonIds: a.PersonIds),
                    statusCode: StatusCodes.Status422UnprocessableEntity),
                _ => Results.Problem("unexpected lookup result", statusCode: StatusCodes.Status500InternalServerError),
            };
        });

        app.MapGet("/health", async (PersonsRepository repo, CancellationToken cancellationToken) =>
        {
            var ok = await repo.PingAsync(cancellationToken).ConfigureAwait(false);
            return ok
                ? Results.Ok(new { status = "healthy" })
                : Results.Json(new { status = "unhealthy" }, statusCode: StatusCodes.Status503ServiceUnavailable);
        });

        app.MapGet("/healthz", () => Results.Text("ok", "text/plain"));

        return app;
    }
}
