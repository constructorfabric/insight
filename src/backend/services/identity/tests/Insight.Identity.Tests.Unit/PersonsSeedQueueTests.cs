using FluentAssertions;
using Insight.Identity.Api.Background;
using Xunit;

namespace Insight.Identity.Tests.Unit;

/// <summary>
/// The bounded-queue contract that the endpoint's 503 path depends on:
/// once the channel is full, <see cref="PersonsSeedQueue.TryEnqueue"/>
/// returns false (never blocks, never drops) so the POST handler can map
/// that to a 503.
/// </summary>
public sealed class PersonsSeedQueueTests
{
    private const int Capacity = 100;

    private static PersonsSeedJob Job() => new(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid());

    [Fact]
    public void Accepts_jobs_up_to_capacity()
    {
        var queue = new PersonsSeedQueue();

        for (var i = 0; i < Capacity; i++)
        {
            queue.TryEnqueue(Job()).Should().BeTrue($"job {i} is within capacity");
        }
    }

    [Fact]
    public void Rejects_once_full_so_caller_can_503()
    {
        var queue = new PersonsSeedQueue();
        for (var i = 0; i < Capacity; i++)
        {
            queue.TryEnqueue(Job());
        }

        // The overflow write does not block or drop — it returns false,
        // which the POST handler turns into 503 + a failed operation row.
        queue.TryEnqueue(Job()).Should().BeFalse();
    }
}
