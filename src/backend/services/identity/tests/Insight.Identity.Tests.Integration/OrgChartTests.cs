using FluentAssertions;
using Insight.Identity.Domain.Services;
using Insight.Identity.Infrastructure.MariaDb;
using MySqlConnector;
using Xunit;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// Integration tests for <c>org_chart</c> reads.
/// Phase 1 of constructorfabric/insight#348 — verifies that
/// <see cref="PersonsRepository.GetCurrentParentsAsync"/> and
/// <see cref="PersonsRepository.GetCurrentChildrenAsync"/> return the
/// CURRENT edges only (<c>valid_to IS NULL</c>), are tenant-scoped, and
/// project the per-source-instance edge granularity through to the
/// domain shape. No Phase-2 API surface yet.
/// </summary>
[Collection(MariaDbCollection.Name)]
public sealed class OrgChartTests : IAsyncLifetime
{
    private static readonly Guid TenantId       = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid OtherTenantId  = Guid.Parse("99999999-9999-9999-9999-999999999999");
    private static readonly Guid BambooSourceId = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    private static readonly Guid ZoomSourceId   = Guid.Parse("dddddddd-dddd-dddd-dddd-dddddddddddd");
    private static readonly Guid SlackSourceId  = Guid.Parse("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee");
    private static readonly Guid AlicePersonId  = Guid.Parse("cccccccc-cccc-cccc-cccc-cccccccccccc");
    private static readonly Guid BobPersonId    = Guid.Parse("ffffffff-ffff-ffff-ffff-ffffffffffff");
    private static readonly Guid CarolPersonId  = Guid.Parse("11111111-1111-1111-1111-111111111111");
    private static readonly Guid AuthorPersonId = Guid.Empty;

    private readonly MariaDbFixture _fixture;
    private PersonsRepository? _repo;

    public OrgChartTests(MariaDbFixture fixture) => _fixture = fixture;

    public async Task InitializeAsync()
    {
        await _fixture.ResetAsync().ConfigureAwait(false);
        _repo = new PersonsRepository(new MariaDbConnectionFactory(
            new Microsoft.Extensions.Options.OptionsWrapper<MariaDbOptions>(
                new MariaDbOptions { ConnectionString = _fixture.ConnectionString })));
    }

    public Task DisposeAsync() => Task.CompletedTask;

    [Fact]
    public async Task GetCurrentParents_returns_one_edge_per_source_instance()
    {
        // Alice reports to Bob in BambooHR and to Carol in Zoom.
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);
        await InsertEdgeAsync("zoom",     ZoomSourceId,   child: AlicePersonId, parent: CarolPersonId).ConfigureAwait(false);

        var parents = await _repo!.GetCurrentParentsAsync(TenantId, AlicePersonId, CancellationToken.None);

        parents.Should().HaveCount(2);
        parents.Should().Contain(e => e.InsightSourceType == "bamboohr" && e.ParentPersonId == BobPersonId);
        parents.Should().Contain(e => e.InsightSourceType == "zoom"     && e.ParentPersonId == CarolPersonId);
    }

    [Fact]
    public async Task GetCurrentParents_returns_empty_when_no_parents()
    {
        var parents = await _repo!.GetCurrentParentsAsync(TenantId, AlicePersonId, CancellationToken.None);
        parents.Should().BeEmpty();
    }

    [Fact]
    public async Task GetCurrentParents_excludes_historical_edges()
    {
        // Alice used to report to Bob (closed) and now reports to Carol (open).
        // Same source instance — only the current edge should come back.
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId,
            validFrom: new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc),
            validTo:   new DateTime(2026, 3, 1, 0, 0, 0, DateTimeKind.Utc)).ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: CarolPersonId,
            validFrom: new DateTime(2026, 3, 1, 0, 0, 0, DateTimeKind.Utc),
            validTo:   null).ConfigureAwait(false);

        var parents = await _repo!.GetCurrentParentsAsync(TenantId, AlicePersonId, CancellationToken.None);

        parents.Should().HaveCount(1);
        parents[0].ParentPersonId.Should().Be(CarolPersonId);
        parents[0].ValidFrom.Should().Be(new DateTime(2026, 3, 1, 0, 0, 0, DateTimeKind.Utc));
    }

    [Fact]
    public async Task GetCurrentParents_is_tenant_scoped()
    {
        // Alice has a parent in our tenant; the same Alice UUID is reused
        // in another tenant's data — must not leak across.
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: CarolPersonId,
            tenantId: OtherTenantId).ConfigureAwait(false);

        var parentsOurs = await _repo!.GetCurrentParentsAsync(TenantId, AlicePersonId, CancellationToken.None);
        parentsOurs.Should().HaveCount(1);
        parentsOurs[0].ParentPersonId.Should().Be(BobPersonId);
    }

    [Fact]
    public async Task GetCurrentChildren_returns_all_direct_reports_across_sources()
    {
        // Bob manages Alice (BambooHR) and Carol (Slack).
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);
        await InsertEdgeAsync("slack",    SlackSourceId,  child: CarolPersonId, parent: BobPersonId).ConfigureAwait(false);

        var children = await _repo!.GetCurrentChildrenAsync(TenantId, BobPersonId, CancellationToken.None);

        children.Should().HaveCount(2);
        children.Should().Contain(e => e.InsightSourceType == "bamboohr" && e.ChildPersonId == AlicePersonId);
        children.Should().Contain(e => e.InsightSourceType == "slack"    && e.ChildPersonId == CarolPersonId);
    }

    [Fact]
    public async Task GetCurrentChildren_returns_empty_when_leaf()
    {
        // Alice has no subordinates. Bob does, but we query Alice.
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);

        var children = await _repo!.GetCurrentChildrenAsync(TenantId, AlicePersonId, CancellationToken.None);
        children.Should().BeEmpty();
    }

    // ── SCD2 history with re-activation (ADR-0010 active intervals) ──

    [Fact]
    public async Task GetCurrentParents_returns_only_open_row_when_child_has_history()
    {
        // The "В scenario" from the design discussion:
        // Alice reports to Bob from T0 until T2 (deactivation), then
        // re-activates at T3 and resumes reporting to Bob. The rebuild
        // produces TWO rows (one historical [T0,T2), one current
        // [T3,NULL)). GetCurrentParentsAsync must surface only the
        // current row — the reader's `valid_to IS NULL` filter is what
        // makes the SCD2 history invisible to consumers that only
        // care about "right now".
        var t0 = new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);
        var t2 = new DateTime(2026, 6, 1, 0, 0, 0, DateTimeKind.Utc);
        var t3 = new DateTime(2026, 8, 1, 0, 0, 0, DateTimeKind.Utc);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId,
            validFrom: t0, validTo: t2).ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId,
            validFrom: t3, validTo: null).ConfigureAwait(false);

        var parents = await _repo!.GetCurrentParentsAsync(TenantId, AlicePersonId, CancellationToken.None);

        parents.Should().HaveCount(1);
        parents[0].ParentPersonId.Should().Be(BobPersonId);
        parents[0].ValidFrom.Should().Be(t3);
    }

    [Fact]
    public async Task GetCurrentParents_returns_empty_when_child_only_has_historical_rows()
    {
        // Alice was deactivated at T2 and never re-activated. The
        // rebuild produces a single closed row [T0,T2). Current-state
        // queries must return empty.
        var t0 = new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);
        var t2 = new DateTime(2026, 6, 1, 0, 0, 0, DateTimeKind.Utc);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId,
            validFrom: t0, validTo: t2).ConfigureAwait(false);

        var parents = await _repo!.GetCurrentParentsAsync(TenantId, AlicePersonId, CancellationToken.None);
        parents.Should().BeEmpty();
    }

    [Fact]
    public async Task GetCurrentParents_excludes_no_parent_sentinel_row()
    {
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: null).ConfigureAwait(false);

        var parents = await _repo!.GetCurrentParentsAsync(TenantId, AlicePersonId, CancellationToken.None);
        parents.Should().BeEmpty();
    }

    [Fact]
    public async Task GetCurrentParents_returns_real_edge_alongside_sentinel_from_other_source()
    {
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);
        await InsertEdgeAsync("slack",    SlackSourceId,  child: AlicePersonId, parent: null).ConfigureAwait(false);

        var parents = await _repo!.GetCurrentParentsAsync(TenantId, AlicePersonId, CancellationToken.None);

        parents.Should().HaveCount(1);
        parents[0].ParentPersonId.Should().Be(BobPersonId);
    }

    [Fact]
    public async Task GetCurrentChildren_excludes_child_whose_latest_row_is_closed()
    {
        // Bob has two direct reports: Alice (deactivated, never
        // re-activated -- closed edge) and Carol (active). The parent
        // query must surface only Carol.
        var t0 = new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);
        var t2 = new DateTime(2026, 6, 1, 0, 0, 0, DateTimeKind.Utc);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId,
            validFrom: t0, validTo: t2).ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: CarolPersonId, parent: BobPersonId,
            validFrom: t0, validTo: null).ConfigureAwait(false);

        var children = await _repo!.GetCurrentChildrenAsync(TenantId, BobPersonId, CancellationToken.None);

        children.Should().HaveCount(1);
        children[0].ChildPersonId.Should().Be(CarolPersonId);
    }

    // ── Seed helpers ──────────────────────────────────────────────────

    private async Task InsertEdgeAsync(
        string sourceType,
        Guid sourceId,
        Guid child,
        Guid? parent,
        DateTime? validFrom = null,
        DateTime? validTo   = null,
        Guid? tenantId      = null)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            INSERT INTO org_chart
                (insight_tenant_id, insight_source_type, insight_source_id,
                 child_person_id, parent_person_id, author_person_id, reason,
                 valid_from, valid_to)
            VALUES (@t, @st, @sid, @c, @p, @a, '', @vf, @vt)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@t",   (tenantId ?? TenantId).ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@st",  sourceType);
        cmd.Parameters.AddWithValue("@sid", sourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@c",   child.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@p",   (object?)parent?.ToByteArray(bigEndian: true) ?? DBNull.Value);
        cmd.Parameters.AddWithValue("@a",   AuthorPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@vf",  validFrom ?? new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc));
        cmd.Parameters.AddWithValue("@vt",  (object?)validTo ?? DBNull.Value);
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }
}
