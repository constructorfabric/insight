using System.Text.Json;
using Insight.Identity.Api.Auth;
using Insight.Identity.Api.Background;
using Insight.Identity.Api.Contracts;
using Insight.Identity.Domain;
using Insight.Identity.Domain.Services;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Routing;
using Microsoft.Extensions.Logging;

namespace Insight.Identity.Api.Endpoints;

/// <summary>
/// Admin-triggered bulk re-seed of <c>persons</c> /
/// <c>account_person_map</c> / <c>org_chart</c> from ClickHouse
/// <c>identity_inputs</c>. POST enqueues an async operation (202 +
/// Location); GET probes its status. Scoped to the caller's tenant.
/// </summary>
public static class PersonsSeedEndpoints
{
    private const string InvalidModeUrn = "urn:insight:error:invalid_mode";
    private const string QueueFullUrn   = "urn:insight:error:seed_queue_full";

    // Phase 1 supports a single mode. The body field is forward-
    // extensible; unknown / null modes default to link-by-email.
    private const string LinkByEmailMode = "link-by-email";

    private static readonly JsonSerializerOptions RequestJsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
    };

    public static IEndpointRouteBuilder MapPersonsSeedEndpoints(this IEndpointRouteBuilder app)
    {
        ArgumentNullException.ThrowIfNull(app);

        app.MapPost("/v1/persons-seed", async (
            PersonsSeedRequest? body,
            HttpContext http,
            CallerAdminCheck admin,
            PersonsSeedQueue queue,
            IOperationsRepository operations,
            ILoggerFactory loggerFactory,
            CancellationToken ct) =>
        {
            var gate = await admin.CheckAsync(http, ct).ConfigureAwait(false);
            if (gate is not AdminCheckResult.IsAdmin) return EndpointHelpers.GateResult(gate);

            var mode = string.IsNullOrWhiteSpace(body?.Mode) ? LinkByEmailMode : body!.Mode!.Trim();
            if (!string.Equals(mode, LinkByEmailMode, StringComparison.Ordinal))
            {
                return Results.Json(new ProblemResponse(
                    Type: InvalidModeUrn,
                    Title: "Bad Request",
                    Status: StatusCodes.Status400BadRequest,
                    Detail: $"unsupported mode '{mode}'; only '{LinkByEmailMode}' is available"),
                    statusCode: StatusCodes.Status400BadRequest);
            }

            var tenantId = EndpointHelpers.ResolveTenant(http)!.Value;
            var callerPersonId = (await EndpointHelpers.ResolveCallerAsync(http, ct).ConfigureAwait(false))!.Value;
            var requestJson = JsonSerializer.Serialize(new PersonsSeedRequest(mode), RequestJsonOptions);

            var operationId = await operations
                .EnqueueAsync(OperationTypes.PersonsSeed, tenantId, callerPersonId, requestJson, ct)
                .ConfigureAwait(false);

            if (!queue.TryEnqueue(new PersonsSeedJob(operationId, tenantId, callerPersonId)))
            {
                // Channel full — fail the row immediately so it isn't a
                // zombie, and tell the caller to retry later.
                await operations
                    .FailAsync(operationId, "seed queue full; retry later", CancellationToken.None)
                    .ConfigureAwait(false);
                return Results.Json(new ProblemResponse(
                    Type: QueueFullUrn,
                    Title: "Service Unavailable",
                    Status: StatusCodes.Status503ServiceUnavailable,
                    Detail: "seed queue is full; retry later"),
                    statusCode: StatusCodes.Status503ServiceUnavailable);
            }

            EndpointHelpers.Audit(loggerFactory, "persons_seed.enqueue",
                ("operation_id", operationId),
                ("mode", mode),
                ("author_person_id", callerPersonId));

            var response = PersonsSeedOperationResponse.Queued(
                operationId, tenantId, callerPersonId, requestJson, DateTime.UtcNow);
            return Results.Accepted($"/v1/persons-seed/{operationId}", response);
        });

        app.MapGet("/v1/persons-seed/{id}", async (
            Guid id,
            HttpContext http,
            CallerAdminCheck admin,
            IOperationsRepository operations,
            CancellationToken ct) =>
        {
            var gate = await admin.CheckAsync(http, ct).ConfigureAwait(false);
            if (gate is not AdminCheckResult.IsAdmin) return EndpointHelpers.GateResult(gate);

            var tenantId = EndpointHelpers.ResolveTenant(http)!.Value;
            var op = await operations.GetByIdAsync(tenantId, id, ct).ConfigureAwait(false);
            if (op is null || !string.Equals(op.OperationType, OperationTypes.PersonsSeed, StringComparison.Ordinal))
            {
                return EndpointHelpers.NotFound("persons_seed_operation", id);
            }
            return Results.Ok(PersonsSeedOperationResponse.From(op));
        });

        app.MapGet("/v1/persons-seed", async (
            HttpContext http,
            CallerAdminCheck admin,
            IOperationsRepository operations,
            string? status,
            int? limit,
            CancellationToken ct) =>
        {
            var gate = await admin.CheckAsync(http, ct).ConfigureAwait(false);
            if (gate is not AdminCheckResult.IsAdmin) return EndpointHelpers.GateResult(gate);

            OperationStatus? statusFilter = status switch
            {
                null or ""  => null,
                "queued"    => OperationStatus.Queued,
                "running"   => OperationStatus.Running,
                "completed" => OperationStatus.Completed,
                "failed"    => OperationStatus.Failed,
                _ => null,
            };

            var tenantId = EndpointHelpers.ResolveTenant(http)!.Value;
            var page = new PageRequest(limit ?? PageRequest.DefaultLimit);
            var result = await operations
                .ListAsync(tenantId, OperationTypes.PersonsSeed, statusFilter, page, ct)
                .ConfigureAwait(false);
            var items = result.Items.Select(PersonsSeedOperationResponse.From).ToList();
            return Results.Ok(new ListResponse<PersonsSeedOperationResponse>(items, result.NextCursor));
        });

        return app;
    }
}
