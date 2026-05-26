using FluentValidation;
using Insight.Identity.Api.Auth;
using Insight.Identity.Api.Contracts;
using Insight.Identity.Domain.Services;
using Insight.Identity.Infrastructure.MariaDb;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Mvc;
using Microsoft.AspNetCore.Routing;
using Microsoft.Extensions.Logging;

namespace Insight.Identity.Api.Endpoints;

/// <summary>
/// CRUD over the <c>visibility</c> table. Admin-only (gated via
/// <see cref="CallerAdminCheck"/>); see ADR-0012.
/// </summary>
public static class VisibilityEndpoints
{
    public static IEndpointRouteBuilder MapVisibilityEndpoints(this IEndpointRouteBuilder app)
    {
        ArgumentNullException.ThrowIfNull(app);

        app.MapPost("/v1/visibility", async (
            CreateVisibilityCommandModel body,
            HttpContext http,
            CallerAdminCheck admin,
            IValidator<CreateVisibilityCommandModel> validator,
            VisibilityRepository repo,
            ILoggerFactory loggerFactory,
            CancellationToken ct) =>
        {
            var gate = await admin.CheckAsync(http, ct).ConfigureAwait(false);
            if (gate is not AdminCheckResult.IsAdmin) return EndpointHelpers.GateResult(gate);

            var validation = await validator.ValidateAsync(body, ct).ConfigureAwait(false);
            if (!validation.IsValid) return EndpointHelpers.ValidationFailure(validation);

            var tenantId = EndpointHelpers.ResolveTenant(http)!.Value;
            var callerPersonId = (await EndpointHelpers.ResolveCallerAsync(http, ct).ConfigureAwait(false))!.Value;
            var id = await repo.InsertAsync(
                tenantId, body.ViewerPersonId, body.ViewedPersonId,
                body.ValidFrom, callerPersonId, body.Reason, ct).ConfigureAwait(false);
            EndpointHelpers.Audit(loggerFactory, "visibility.create",
                ("visibility_id", id),
                ("viewer_person_id", body.ViewerPersonId),
                ("viewed_person_id", body.ViewedPersonId),
                ("author_person_id", callerPersonId));
            var created = await repo.GetByIdAsync(tenantId, id, ct).ConfigureAwait(false);
            return Results.Created($"/v1/visibility/{id:D}", VisibilityResponse.From(created!));
        });

        app.MapGet("/v1/visibility", async (
            HttpContext http,
            CallerAdminCheck admin,
            VisibilityRepository repo,
            Guid? viewer,
            Guid? viewed,
            bool? active,
            int? limit,
            CancellationToken ct) =>
        {
            var gate = await admin.CheckAsync(http, ct).ConfigureAwait(false);
            if (gate is not AdminCheckResult.IsAdmin) return EndpointHelpers.GateResult(gate);

            var tenantId = EndpointHelpers.ResolveTenant(http)!.Value;
            var page = new PageRequest(limit ?? PageRequest.DefaultLimit);
            var result = await repo.ListAsync(
                tenantId, viewer, viewed, active ?? false, page, ct).ConfigureAwait(false);
            var items = result.Items.Select(VisibilityResponse.From).ToList();
            return Results.Ok(new ListResponse<VisibilityResponse>(items, result.NextCursor));
        });

        app.MapDelete("/v1/visibility/{id}", async (
            Guid id,
            [FromBody] RevokeReasonModel? body,
            HttpContext http,
            CallerAdminCheck admin,
            VisibilityRepository repo,
            ILoggerFactory loggerFactory,
            CancellationToken ct) =>
        {
            var gate = await admin.CheckAsync(http, ct).ConfigureAwait(false);
            if (gate is not AdminCheckResult.IsAdmin) return EndpointHelpers.GateResult(gate);

            var tenantId = EndpointHelpers.ResolveTenant(http)!.Value;
            var existing = await repo.GetByIdAsync(tenantId, id, ct).ConfigureAwait(false);
            if (existing is null) return EndpointHelpers.NotFound("visibility", id);

            var rows = await repo.SoftDeleteAsync(tenantId, id, body?.Reason, ct).ConfigureAwait(false);
            EndpointHelpers.Audit(loggerFactory, "visibility.revoke",
                ("visibility_id", id),
                ("rows_affected", rows),
                ("author_person_id", (await EndpointHelpers.ResolveCallerAsync(http, ct).ConfigureAwait(false))!.Value));
            return Results.NoContent();
        });

        return app;
    }
}
