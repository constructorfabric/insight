using System.Text.Json;
using Insight.Identity.Domain;

namespace Insight.Identity.Api.Contracts;

/// <summary>
/// Body of <c>POST /v1/persons-seed</c>. <see cref="Mode"/> is forward-
/// extensible; Phase 1 accepts only <c>link-by-email</c>.
/// </summary>
public sealed record PersonsSeedRequest(string? Mode);

/// <summary>
/// Response shape for <c>GET /v1/persons-seed/{id}</c> and the items of
/// the list endpoint. Mirrors an <see cref="Operation"/> row; the
/// <c>request</c> and <c>summary</c> JSON columns are surfaced as parsed
/// JSON elements so the wire shape stays structured (not double-encoded
/// strings).
/// </summary>
public sealed record PersonsSeedOperationResponse(
    Guid OperationId,
    string OperationType,
    string Status,
    Guid InsightTenantId,
    Guid AuthorPersonId,
    JsonElement? Request,
    JsonElement? Summary,
    string? ErrorMessage,
    DateTime StartedAt,
    DateTime? CompletedAt)
{
    public static PersonsSeedOperationResponse From(Operation op) => new(
        OperationId:     op.OperationId,
        OperationType:   op.OperationType,
        Status:          StatusToString(op.Status),
        InsightTenantId: op.InsightTenantId,
        AuthorPersonId:  op.AuthorPersonId,
        Request:         ParseOrNull(op.RequestJson),
        Summary:         ParseOrNull(op.SummaryJson),
        ErrorMessage:    op.ErrorMessage,
        StartedAt:       op.StartedAt,
        CompletedAt:     op.CompletedAt);

    /// <summary>
    /// The just-enqueued shape for the <c>202 Accepted</c> body, built
    /// from the fields the POST handler already holds — avoids a second
    /// round-trip to re-read the row that was just inserted.
    /// </summary>
    public static PersonsSeedOperationResponse Queued(
        Guid operationId, Guid tenantId, Guid authorPersonId, string? requestJson, DateTime startedAt) => new(
        OperationId:     operationId,
        OperationType:   OperationTypes.PersonsSeed,
        Status:          StatusToString(OperationStatus.Queued),
        InsightTenantId: tenantId,
        AuthorPersonId:  authorPersonId,
        Request:         ParseOrNull(requestJson),
        Summary:         null,
        ErrorMessage:    null,
        StartedAt:       startedAt,
        CompletedAt:     null);

    private static JsonElement? ParseOrNull(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }
        using var doc = JsonDocument.Parse(json);
        return doc.RootElement.Clone();
    }

    private static string StatusToString(OperationStatus status) => status switch
    {
        OperationStatus.Queued    => "queued",
        OperationStatus.Running   => "running",
        OperationStatus.Completed => "completed",
        OperationStatus.Failed    => "failed",
        _ => "unknown",
    };
}
