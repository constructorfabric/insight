using System.Text.Json;
using Insight.Identity.Domain;
using Insight.Identity.Domain.Services;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;

namespace Insight.Identity.Api.Background;

/// <summary>
/// Background drainer for the <c>persons-seed</c> queue. On startup it
/// sweeps zombie operations left <c>queued</c>/<c>running</c> by a prior
/// process, then processes jobs one at a time: flip the row to
/// <c>running</c>, run <see cref="PersonsSeedService"/>, record
/// completion or failure. Single concurrency by design — seeds rebuild
/// whole tenant caches and must not overlap.
/// </summary>
public sealed class PersonsSeedWorker : BackgroundService
{
    // Operations older than this still in queued/running on startup are
    // assumed orphaned by a pod restart. Edge case: a job queued just
    // before a restart (row only minutes old) is younger than the cutoff
    // and so is NOT swept — its in-memory channel entry is gone, so it
    // sits in `queued` until a later restart occurs after the 1h window
    // elapses. Accepted trade-off of the in-memory queue; a durable queue
    // would remove it (see follow-up).
    private static readonly TimeSpan ZombieCutoff = TimeSpan.FromHours(1);

    private static readonly JsonSerializerOptions SummaryJsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
    };

    private readonly PersonsSeedQueue _queue;
    private readonly PersonsSeedService _seed;
    private readonly IOperationsRepository _operations;
    private readonly ILogger<PersonsSeedWorker> _log;

    public PersonsSeedWorker(
        PersonsSeedQueue queue,
        PersonsSeedService seed,
        IOperationsRepository operations,
        ILogger<PersonsSeedWorker> log)
    {
        _queue = queue;
        _seed = seed;
        _operations = operations;
        _log = log;
    }

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        await SweepZombiesAsync(stoppingToken).ConfigureAwait(false);

        await foreach (var job in _queue.ReadAllAsync(stoppingToken).ConfigureAwait(false))
        {
            await ProcessAsync(job, stoppingToken).ConfigureAwait(false);
        }
    }

    private async Task SweepZombiesAsync(CancellationToken cancellationToken)
    {
        try
        {
            var swept = await _operations
                .SweepZombiesAsync(DateTime.UtcNow - ZombieCutoff, cancellationToken)
                .ConfigureAwait(false);
            if (swept > 0)
            {
                LogZombiesSwept(_log, swept, null);
            }
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            LogSweepFailed(_log, ex);
        }
    }

    private async Task ProcessAsync(PersonsSeedJob job, CancellationToken cancellationToken)
    {
        // TryStart returns false if the row is no longer queued — a
        // double-enqueue or a row already swept; skip silently.
        var started = await _operations.TryStartAsync(job.OperationId, cancellationToken).ConfigureAwait(false);
        if (!started)
        {
            return;
        }

        try
        {
            var summary = await _seed
                .RunAsync(job.TenantId, job.AuthorPersonId, cancellationToken)
                .ConfigureAwait(false);
            var summaryJson = JsonSerializer.Serialize(summary, SummaryJsonOptions);
            await _operations.CompleteAsync(job.OperationId, summaryJson, cancellationToken).ConfigureAwait(false);
            LogCompleted(_log, job.OperationId, summary.ObservationsInserted, null);
        }
        catch (OperationCanceledException)
        {
            // Shutdown — leave the row running; the next pod's startup
            // sweep flips it to failed.
            throw;
        }
        catch (Exception ex)
        {
            await _operations.FailAsync(job.OperationId, ex.Message, CancellationToken.None).ConfigureAwait(false);
            LogFailed(_log, job.OperationId, ex);
        }
    }

    private static readonly Action<ILogger, int, Exception?> LogZombiesSwept =
        LoggerMessage.Define<int>(LogLevel.Warning, new EventId(1, nameof(LogZombiesSwept)),
            "persons-seed: swept {Count} zombie operation(s) on startup");

    private static readonly Action<ILogger, Exception?> LogSweepFailed =
        LoggerMessage.Define(LogLevel.Error, new EventId(2, nameof(LogSweepFailed)),
            "persons-seed: zombie sweep failed");

    private static readonly Action<ILogger, Guid, int, Exception?> LogCompleted =
        LoggerMessage.Define<Guid, int>(LogLevel.Information, new EventId(3, nameof(LogCompleted)),
            "persons-seed: operation {OperationId} completed, {Inserted} observation(s) inserted");

    private static readonly Action<ILogger, Guid, Exception?> LogFailed =
        LoggerMessage.Define<Guid>(LogLevel.Error, new EventId(4, nameof(LogFailed)),
            "persons-seed: operation {OperationId} failed");
}
