using FluentAssertions;
using Insight.Identity.Domain.Services;
using Xunit;

namespace Insight.Identity.Tests.Unit;

public sealed class PersonsSeedServiceTests
{
    private static readonly Guid Tenant = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid Author = Guid.Parse("dddddddd-dddd-dddd-dddd-dddddddddddd");
    private static readonly Guid Source = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    private static readonly Guid Minted = Guid.Parse("33333333-3333-3333-3333-333333333333");
    private static readonly DateTime T0 = new(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);

    private static IdentityInputRow Row(string account, string valueType, string value, bool isDelete = false, int minute = 0)
        => new(Tenant, "bamboohr", Source, account, valueType, value, T0.AddMinutes(minute), isDelete);

    [Fact]
    public async Task Mints_new_person_for_unknown_active_account()
    {
        var inputs = new FakeInputs(
            Row("acc-1", "email", "new@x.io"),
            Row("acc-1", "display_name", "New Person"));
        var store = new FakeStore();
        var svc = new PersonsSeedService(inputs, store, () => Minted);

        var summary = await svc.RunAsync(Tenant, Author, CancellationToken.None);

        summary.AccountsRead.Should().Be(1);
        summary.AccountsMintedNew.Should().Be(1);
        summary.ObservationsInserted.Should().Be(2);
        store.Inserted.Should().OnlyContain(r => r.PersonId == Minted);
        store.Inserted.Should().OnlyContain(r => r.Reason == string.Empty); // minted → blank reason
        store.Applied.Should().BeTrue();
    }

    [Fact]
    public async Task Links_to_existing_email_person_with_auto_seed_link_reason()
    {
        var existing = Guid.Parse("22222222-2222-2222-2222-222222222222");
        var inputs = new FakeInputs(Row("acc-1", "email", "a@x.io"));
        var store = new FakeStore { EmailToPerson = { ["a@x.io"] = existing } };
        var svc = new PersonsSeedService(inputs, store, () => Minted);

        var summary = await svc.RunAsync(Tenant, Author, CancellationToken.None);

        summary.AccountsLinkedByEmail.Should().Be(1);
        summary.AccountsMintedNew.Should().Be(0);
        store.Inserted.Should().OnlyContain(r => r.PersonId == existing);
        store.Inserted.Should().OnlyContain(r => r.Reason == PersonAssignmentResolver.AutoSeedLinkReason);
    }

    [Fact]
    public async Task Reuses_known_account_binding_with_blank_reason()
    {
        var known = Guid.Parse("11111111-1111-1111-1111-111111111111");
        var inputs = new FakeInputs(Row("acc-1", "email", "a@x.io"));
        var store = new FakeStore
        {
            KnownAccounts = { [new SourceAccountKey("bamboohr", Source, "acc-1")] = known },
        };
        var svc = new PersonsSeedService(inputs, store, () => Minted);

        var summary = await svc.RunAsync(Tenant, Author, CancellationToken.None);

        summary.AccountsReusedKnown.Should().Be(1);
        store.Inserted.Should().OnlyContain(r => r.PersonId == known);
        store.Inserted.Should().OnlyContain(r => r.Reason == string.Empty);
    }

    [Fact]
    public async Task Closed_account_with_no_email_match_is_skipped_no_rows_written()
    {
        // Latest observation is a DELETE → account closed → no email match → skip.
        var inputs = new FakeInputs(
            Row("acc-1", "email", "gone@x.io", isDelete: true, minute: 10),
            Row("acc-1", "email", "gone@x.io", minute: 0));
        var store = new FakeStore();
        var svc = new PersonsSeedService(inputs, store, () => Minted);

        var summary = await svc.RunAsync(Tenant, Author, CancellationToken.None);

        summary.AccountsSkippedClosed.Should().Be(1);
        summary.AccountsMintedNew.Should().Be(0);
        store.Inserted.Should().BeEmpty();
    }

    [Fact]
    public async Task Delete_rows_never_become_observations()
    {
        // Active account (latest row is the UPSERT email at minute 10),
        // with an older DELETE status row at minute 5. Reader yields
        // latest-first. The account mints; the email UPSERT is written
        // but the DELETE status row is signal-only and never persisted.
        var inputs = new FakeInputs(
            Row("acc-1", "email", "a@x.io", minute: 10),
            Row("acc-1", "status", "Terminated", isDelete: true, minute: 5));
        var store = new FakeStore();
        var svc = new PersonsSeedService(inputs, store, () => Minted);

        await svc.RunAsync(Tenant, Author, CancellationToken.None);

        store.Inserted.Should().ContainSingle(r => r.ValueType == "email");
        store.Inserted.Should().NotContain(r => r.ValueType == "status");
    }

    private sealed class FakeInputs : IIdentityInputsReader
    {
        private readonly IReadOnlyList<IdentityInputRow> _rows;
        public FakeInputs(params IdentityInputRow[] rows) => _rows = rows;

        public async IAsyncEnumerable<IdentityInputRow> StreamAsync(
            Guid tenantId,
            [System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken)
        {
            foreach (var row in _rows)
            {
                cancellationToken.ThrowIfCancellationRequested();
                yield return row;
                await Task.Yield();
            }
        }
    }

    private sealed class FakeStore : IPersonsSeedStore
    {
        public Dictionary<SourceAccountKey, Guid> KnownAccounts { get; } = new();
        public Dictionary<string, Guid> EmailToPerson { get; } = new(StringComparer.OrdinalIgnoreCase);
        public List<PersonObservationRow> Inserted { get; } = new();
        public bool Applied { get; private set; }

        public Task<IReadOnlyDictionary<SourceAccountKey, Guid>> GetKnownAccountBindingsAsync(Guid tenantId, CancellationToken ct)
            => Task.FromResult<IReadOnlyDictionary<SourceAccountKey, Guid>>(KnownAccounts);

        public Task<IReadOnlyDictionary<string, Guid>> GetLatestEmailToPersonAsync(Guid tenantId, CancellationToken ct)
            => Task.FromResult<IReadOnlyDictionary<string, Guid>>(EmailToPerson);

        public Task<SeedApplyResult> ApplyAsync(Guid tenantId, IReadOnlyList<PersonObservationRow> rows, CancellationToken ct)
        {
            Applied = true;
            Inserted.AddRange(rows);
            return Task.FromResult(new SeedApplyResult(rows.Count, OrgChartEdgesRebuilt: 0));
        }
    }
}
