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
/// CRUD over the <c>person_roles</c> junction. Admin-only; revoke
/// refuses to remove the last active admin assignment in a tenant
/// (lockout protection). See ADR-0014.
/// </summary>
public static class PersonRolesEndpoints
{
    private const string LastAdminProtectedUrn = "urn:insight:error:last_admin_protected";

    public static IEndpointRouteBuilder MapPersonRoleEndpoints(this IEndpointRouteBuilder app)
    {
        ArgumentNullException.ThrowIfNull(app);

        app.MapPost("/v1/person-roles", async (
            CreatePersonRoleCommandModel body,
            HttpContext http,
            CallerAdminCheck admin,
            IValidator<CreatePersonRoleCommandModel> validator,
            RolesRepository repo,
            ILoggerFactory loggerFactory,
            CancellationToken ct) =>
        {
            var gate = await admin.CheckAsync(http, ct).ConfigureAwait(false);
            if (gate is not AdminCheckResult.IsAdmin) return EndpointHelpers.GateResult(gate);

            var validation = await validator.ValidateAsync(body, ct).ConfigureAwait(false);
            if (!validation.IsValid) return EndpointHelpers.ValidationFailure(validation);

            var tenantId = EndpointHelpers.ResolveTenant(http)!.Value;
            var callerPersonId = (await EndpointHelpers.ResolveCallerAsync(http, ct).ConfigureAwait(false))!.Value;
            var id = await repo.InsertPersonRoleAsync(
                tenantId, body.PersonId, body.RoleId, body.ValidFrom,
                callerPersonId, body.Reason, ct).ConfigureAwait(false);
            EndpointHelpers.Audit(loggerFactory, "person_roles.create",
                ("person_role_id", id),
                ("person_id", body.PersonId),
                ("role_id", body.RoleId),
                ("author_person_id", callerPersonId));
            var created = await repo.GetPersonRoleByIdAsync(tenantId, id, ct).ConfigureAwait(false);
            return Results.Created($"/v1/person-roles/{id:D}", PersonRoleResponse.From(created!));
        });

        app.MapGet("/v1/person-roles", async (
            HttpContext http,
            CallerAdminCheck admin,
            RolesRepository repo,
            Guid? person,
            Guid? role,
            bool? active,
            int? limit,
            CancellationToken ct) =>
        {
            var gate = await admin.CheckAsync(http, ct).ConfigureAwait(false);
            if (gate is not AdminCheckResult.IsAdmin) return EndpointHelpers.GateResult(gate);

            var tenantId = EndpointHelpers.ResolveTenant(http)!.Value;
            var page = new PageRequest(limit ?? PageRequest.DefaultLimit);
            var result = await repo.ListAsync(
                tenantId, person, role, active ?? false, page, ct).ConfigureAwait(false);
            var items = result.Items.Select(PersonRoleResponse.From).ToList();
            return Results.Ok(new ListResponse<PersonRoleResponse>(items, result.NextCursor));
        });

        app.MapDelete("/v1/person-roles/{id}", async (
            Guid id,
            [FromBody] RevokeReasonModel? body,
            HttpContext http,
            CallerAdminCheck admin,
            RolesRepository repo,
            ILoggerFactory loggerFactory,
            CancellationToken ct) =>
        {
            var gate = await admin.CheckAsync(http, ct).ConfigureAwait(false);
            if (gate is not AdminCheckResult.IsAdmin) return EndpointHelpers.GateResult(gate);

            // Pre-fetch for audit metadata (person_id / role_id /
            // tenant); not part of the guard. The atomic UPDATE below
            // owns the last-admin check — two concurrent admin revokes
            // can no longer both slip past a separate COUNT query.
            var tenantId = EndpointHelpers.ResolveTenant(http)!.Value;
            var existing = await repo.GetPersonRoleByIdAsync(tenantId, id, ct).ConfigureAwait(false);
            if (existing is null || existing.ValidTo is not null)
            {
                return EndpointHelpers.NotFound("person_role", id);
            }

            var rowsAffected = await repo
                .TrySoftDeletePersonRoleProtectingLastAdminAsync(id, Roles.Admin, body?.Reason, ct)
                .ConfigureAwait(false);

            if (rowsAffected == 1)
            {
                EndpointHelpers.Audit(loggerFactory, "person_roles.revoke",
                    ("person_role_id", id),
                    ("person_id", existing.PersonId),
                    ("role_id", existing.RoleId),
                    ("author_person_id", (await EndpointHelpers.ResolveCallerAsync(http, ct).ConfigureAwait(false))!.Value));
                return Results.NoContent();
            }

            // rowsAffected == 0: either the row was revoked between
            // pre-fetch and UPDATE (treat as 404) or the last-admin
            // guard fired (422). A second read tells us which.
            var refetched = await repo.GetPersonRoleByIdAsync(tenantId, id, ct).ConfigureAwait(false);
            if (refetched is null || refetched.ValidTo is not null)
            {
                return EndpointHelpers.NotFound("person_role", id);
            }
            return Results.Json(new ProblemResponse(
                Type: LastAdminProtectedUrn,
                Title: "Unprocessable Entity",
                Status: StatusCodes.Status422UnprocessableEntity,
                Detail: "cannot revoke the last active admin assignment in this tenant"),
                statusCode: StatusCodes.Status422UnprocessableEntity);
        });

        return app;
    }
}
