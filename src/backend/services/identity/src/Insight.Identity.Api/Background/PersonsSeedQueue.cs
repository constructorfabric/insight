using System.Threading.Channels;

namespace Insight.Identity.Api.Background;

/// <summary>
/// In-process queue of accepted <c>persons-seed</c> jobs. The POST
/// handler enqueues a job after writing the <c>operations</c> row; the
/// <see cref="PersonsSeedWorker"/> drains it. The job carries the
/// tenant + author resolved at request time so the worker need not
/// re-read the operation row to run the seed.
/// </summary>
// CA1711: the "Queue" suffix is descriptive here — this is a job queue,
// not a System.Collections Queue<T> derivative. Renaming would be less
// clear than the rule's intent warrants.
#pragma warning disable CA1711
public sealed class PersonsSeedQueue
#pragma warning restore CA1711
{
    // Bounded so a misbehaving caller cannot enqueue unbounded work.
    // Seed runs are rare (admin-triggered); 100 in-flight is generous.
    // FullMode is moot here: only TryWrite is used (see TryEnqueue), and
    // TryWrite returns false on a full channel regardless of FullMode —
    // it never drops or blocks. Wait is just the harmless default; the
    // 503-on-full path relies on the false return, not on this setting.
    private readonly Channel<PersonsSeedJob> _channel =
        Channel.CreateBounded<PersonsSeedJob>(new BoundedChannelOptions(100)
        {
            FullMode = BoundedChannelFullMode.Wait,
            SingleReader = true,
        });

    /// <summary>
    /// Enqueue a job. Returns <c>false</c> if the queue is full (caller
    /// maps that to 503) — <c>TryWrite</c> never blocks the request
    /// thread, so the channel's <c>FullMode</c> (which only governs the
    /// blocking <c>WriteAsync</c> path) does not apply here.
    /// </summary>
    public bool TryEnqueue(PersonsSeedJob job) => _channel.Writer.TryWrite(job);

    /// <summary>Async stream the worker reads until shutdown.</summary>
    public IAsyncEnumerable<PersonsSeedJob> ReadAllAsync(CancellationToken cancellationToken) =>
        _channel.Reader.ReadAllAsync(cancellationToken);
}

/// <summary>One queued seed job — all fields resolved at POST time.</summary>
public sealed record PersonsSeedJob(Guid OperationId, Guid TenantId, Guid AuthorPersonId);
