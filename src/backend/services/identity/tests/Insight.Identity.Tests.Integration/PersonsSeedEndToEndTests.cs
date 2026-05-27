using System.Runtime.CompilerServices;
using FluentAssertions;
using Insight.Identity.Domain.Services;
using Insight.Identity.Infrastructure.MariaDb;
using Microsoft.Extensions.Options;
using MySqlConnector;
using Xunit;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// End-to-end <see cref="PersonsSeedService"/> against a real MariaDB
/// (Testcontainers) with an in-memory <see cref="IIdentityInputsReader"/>
/// standing in for ClickHouse. Exercises the apply path and the ported
/// rebuild SQL (account_person_map + the org_chart active-interval /
/// parent_email→email JOIN). The ClickHouse client itself is covered
/// only at the wiring level — the algorithm lives in C# and is what
/// these tests pin down.
/// </summary>
[Collection(MariaDbCollection.Name)]
public sealed class PersonsSeedEndToEndTests : IAsyncLifetime
{
    private static readonly Guid Tenant = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid Author = Guid.Parse("dddddddd-dddd-dddd-dddd-dddddddddddd");
    private static readonly Guid Source = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    private static readonly DateTime T0 = new(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);

    private readonly MariaDbFixture _fixture;
    private PersonsSeedRepository _store = null!;

    public PersonsSeedEndToEndTests(MariaDbFixture fixture) => _fixture = fixture;

    public async Task InitializeAsync()
    {
        await _fixture.ResetAsync().ConfigureAwait(false);
        _store = new PersonsSeedRepository(new MariaDbConnectionFactory(
            Options.Create(new MariaDbOptions { ConnectionString = _fixture.ConnectionString })));
    }

    public Task DisposeAsync() => Task.CompletedTask;

    private static IdentityInputRow Row(string account, string valueType, string value, int minute = 0, bool isDelete = false)
        => new(Tenant, "bamboohr", Source, account, valueType, value, T0.AddMinutes(minute), isDelete);

    [Fact]
    public async Task Seeds_two_accounts_and_builds_org_chart_edge_from_parent_email()
    {
        // boss@x.io is account 100; alice (200) reports to boss via parent_email.
        var inputs = new FakeInputs(
            Row("100", "email", "boss@x.io"),
            Row("100", "display_name", "Boss Person"),
            Row("100", "status", "Active"),
            Row("200", "email", "alice@x.io"),
            Row("200", "display_name", "Alice Person"),
            Row("200", "status", "Active"),
            Row("200", "parent_email", "boss@x.io"));
        var svc = new PersonsSeedService(inputs, _store);

        var summary = await svc.RunAsync(Tenant, Author, default);

        summary.AccountsRead.Should().Be(2);
        summary.AccountsMintedNew.Should().Be(2);
        summary.ObservationsInserted.Should().BeGreaterThan(0);

        // persons: both accounts have an 'id' binding (written by the
        // seed? no — the seed writes the observations it streamed; 'id'
        // is emitted by the connector macro, not present in this fake
        // input). We assert on email observations instead.
        (await CountAsync("SELECT COUNT(*) FROM persons WHERE value_type='email' AND insight_tenant_id=@t"))
            .Should().Be(2);

        // org_chart: one edge, alice(200)'s person → boss(100)'s person.
        var edges = await CountAsync("SELECT COUNT(*) FROM org_chart WHERE insight_tenant_id=@t AND valid_to IS NULL");
        edges.Should().Be(1);
        summary.OrgChartEdgesRebuilt.Should().Be(1);
    }

    [Fact]
    public async Task Rerun_is_idempotent_no_duplicate_observations()
    {
        var inputs = new FakeInputs(
            Row("100", "email", "boss@x.io"),
            Row("100", "display_name", "Boss Person"));
        var svc = new PersonsSeedService(inputs, _store);

        await svc.RunAsync(Tenant, Author, default);
        var afterFirst = await CountAsync("SELECT COUNT(*) FROM persons WHERE insight_tenant_id=@t");

        // Second run with identical inputs: INSERT IGNORE swallows the
        // duplicate observations (same created_at/source/value_type).
        var svc2 = new PersonsSeedService(new FakeInputs(
            Row("100", "email", "boss@x.io"),
            Row("100", "display_name", "Boss Person")), _store);
        var second = await svc2.RunAsync(Tenant, Author, default);
        var afterSecond = await CountAsync("SELECT COUNT(*) FROM persons WHERE insight_tenant_id=@t");

        afterSecond.Should().Be(afterFirst, "INSERT IGNORE on the unique key makes a re-seed a no-op for persons");
        second.ObservationsInserted.Should().Be(0, "the re-seed inserted no NET-NEW rows — all were duplicates");
    }

    [Fact]
    public async Task Closed_account_with_no_email_match_writes_nothing()
    {
        // Reader yields latest-first; the DELETE (minute 10) comes before
        // the original UPSERT (minute 0), so the account reads as closed.
        var inputs = new FakeInputs(
            Row("300", "email", "ghost@x.io", minute: 10, isDelete: true),
            Row("300", "email", "ghost@x.io", minute: 0));
        var svc = new PersonsSeedService(inputs, _store);

        var summary = await svc.RunAsync(Tenant, Author, default);

        summary.AccountsSkippedClosed.Should().Be(1);
        (await CountAsync("SELECT COUNT(*) FROM persons WHERE insight_tenant_id=@t")).Should().Be(0);
    }

    private async Task<long> CountAsync(string sql)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@t", Tenant.ToByteArray(bigEndian: true));
        var raw = await cmd.ExecuteScalarAsync().ConfigureAwait(false);
        return Convert.ToInt64(raw, System.Globalization.CultureInfo.InvariantCulture);
    }

    private sealed class FakeInputs : IIdentityInputsReader
    {
        private readonly IReadOnlyList<IdentityInputRow> _rows;
        public FakeInputs(params IdentityInputRow[] rows) => _rows = rows;

        public async IAsyncEnumerable<IdentityInputRow> StreamAsync(
            Guid tenantId,
            [EnumeratorCancellation] CancellationToken cancellationToken)
        {
            foreach (var row in _rows)
            {
                cancellationToken.ThrowIfCancellationRequested();
                yield return row;
                await Task.Yield();
            }
        }
    }
}
