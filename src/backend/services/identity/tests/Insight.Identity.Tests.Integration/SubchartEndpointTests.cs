using System.Net;
using System.Text.Json;
using FluentAssertions;
using MySqlConnector;
using Xunit;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// End-to-end tests for <c>GET /v1/subchart/{person_id}?depth=N</c>
/// (#348 Phase 3). Seeds the tree
/// <c>Carol → Bob → {Alice, Dave}</c> plus an outsider with no edges.
/// Each test wires the visibility row it needs on top of that fixture.
/// </summary>
[Collection(MariaDbCollection.Name)]
public sealed class SubchartEndpointTests : IAsyncLifetime
{
    private static readonly Guid TenantId          = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid OtherTenantId     = Guid.Parse("ffffffff-ffff-ffff-ffff-ffffffffffff");
    private static readonly Guid BambooSourceId    = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    private static readonly Guid CarolPersonId     = Guid.Parse("11111111-1111-1111-1111-111111111111");
    private static readonly Guid BobPersonId       = Guid.Parse("22222222-2222-2222-2222-222222222222");
    private static readonly Guid AlicePersonId     = Guid.Parse("33333333-3333-3333-3333-333333333333");
    private static readonly Guid DavePersonId      = Guid.Parse("44444444-4444-4444-4444-444444444444");
    private static readonly Guid OutsiderPersonId  = Guid.Parse("55555555-5555-5555-5555-555555555555");
    private static readonly Guid AuthorPersonId    = Guid.Empty;

    private readonly MariaDbFixture _fixture;

    public SubchartEndpointTests(MariaDbFixture fixture) => _fixture = fixture;

    public async Task InitializeAsync()
    {
        await _fixture.ResetAsync().ConfigureAwait(false);
        await SeedPersonAsync(CarolPersonId,    "carol@example.com",    "Carol Lee").ConfigureAwait(false);
        await SeedPersonAsync(BobPersonId,      "bob@example.com",      "Jones, Bob").ConfigureAwait(false);
        await SeedPersonAsync(AlicePersonId,    "alice@example.com",    "Alice Smith").ConfigureAwait(false);
        await SeedPersonAsync(DavePersonId,     "dave@example.com",     "Dave Ng").ConfigureAwait(false);
        await SeedPersonAsync(OutsiderPersonId, "outsider@example.com", "Out Sider").ConfigureAwait(false);
        await InsertEdgeAsync(child: BobPersonId,   parent: CarolPersonId).ConfigureAwait(false);
        await InsertEdgeAsync(child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);
        await InsertEdgeAsync(child: DavePersonId,  parent: BobPersonId).ConfigureAwait(false);
    }

    public Task DisposeAsync() => Task.CompletedTask;

    [Fact]
    public async Task Subchart_root_self_returns_root_with_subordinates()
    {
        // Bob queries own subchart — self short-circuit lets him see
        // himself, descent through org_chart picks up Alice + Dave.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: BobPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync($"/v1/subchart/{BobPersonId:D}").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var root = doc.GetProperty("root");
        root.GetProperty("person_id").GetGuid().Should().Be(BobPersonId);
        root.GetProperty("display_name").GetString().Should().Be("Jones, Bob");
        var directReports = root.GetProperty("subordinates").EnumerateArray()
            .Select(s => s.GetProperty("person_id").GetGuid()).ToArray();
        directReports.Should().BeEquivalentTo(new[] { AlicePersonId, DavePersonId });
    }

    [Fact]
    public async Task Subchart_root_visible_via_visibility_grant_returns_full_tree()
    {
        // Outsider has no org_chart link but holds an active grant on
        // Carol — visible-set CTE folds Carol AND her descendants into
        // the outsider's visible set; subchart returns the full subtree.
        await InsertVisibilityAsync(viewer: OutsiderPersonId, viewed: CarolPersonId).ConfigureAwait(false);
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync($"/v1/subchart/{CarolPersonId:D}").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var root = doc.GetProperty("root");
        root.GetProperty("person_id").GetGuid().Should().Be(CarolPersonId);
        var lvl1 = root.GetProperty("subordinates").EnumerateArray().ToArray();
        lvl1.Should().ContainSingle();
        lvl1[0].GetProperty("person_id").GetGuid().Should().Be(BobPersonId);
        var lvl2 = lvl1[0].GetProperty("subordinates").EnumerateArray()
            .Select(s => s.GetProperty("person_id").GetGuid()).ToArray();
        lvl2.Should().BeEquivalentTo(new[] { AlicePersonId, DavePersonId });
    }

    [Fact]
    public async Task Subchart_root_invisible_returns_404()
    {
        // Outsider has no grant, no org_chart link → can't see Carol.
        // 404 in the same shape as "not found" so existence doesn't leak.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync($"/v1/subchart/{CarolPersonId:D}").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.NotFound);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:person_not_found");
    }

    [Fact]
    public async Task Subchart_depth_zero_returns_only_root()
    {
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: CarolPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync($"/v1/subchart/{CarolPersonId:D}?depth=0").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var root = doc.GetProperty("root");
        root.GetProperty("person_id").GetGuid().Should().Be(CarolPersonId);
        root.GetProperty("subordinates").GetArrayLength().Should().Be(0);
    }

    [Fact]
    public async Task Subchart_depth_one_returns_root_plus_direct_reports()
    {
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: CarolPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync($"/v1/subchart/{CarolPersonId:D}?depth=1").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var root = doc.GetProperty("root");
        var lvl1 = root.GetProperty("subordinates").EnumerateArray().ToArray();
        lvl1.Should().ContainSingle();
        lvl1[0].GetProperty("person_id").GetGuid().Should().Be(BobPersonId);
        // depth=1 stops at Bob; Alice + Dave (depth=2) are pruned.
        lvl1[0].GetProperty("subordinates").GetArrayLength().Should().Be(0);
    }

    [Fact]
    public async Task Subchart_depth_two_returns_root_plus_two_levels()
    {
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: CarolPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync($"/v1/subchart/{CarolPersonId:D}?depth=2").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var lvl1 = doc.GetProperty("root").GetProperty("subordinates").EnumerateArray().ToArray();
        var lvl2 = lvl1[0].GetProperty("subordinates").EnumerateArray()
            .Select(s => s.GetProperty("person_id").GetGuid()).ToArray();
        lvl2.Should().BeEquivalentTo(new[] { AlicePersonId, DavePersonId });
    }

    [Fact]
    public async Task Subchart_no_depth_param_returns_full_tree()
    {
        // No ?depth → unlimited (MariaDB's cte_max_recursion_depth = 1000
        // is the hard ceiling; cycles are prevented by the seeder, not
        // this query). Returns every node under Carol.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: CarolPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync($"/v1/subchart/{CarolPersonId:D}").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var lvl1 = doc.GetProperty("root").GetProperty("subordinates").EnumerateArray().ToArray();
        lvl1.Should().ContainSingle();
        var lvl2 = lvl1[0].GetProperty("subordinates").EnumerateArray()
            .Select(s => s.GetProperty("person_id").GetGuid()).ToArray();
        lvl2.Should().BeEquivalentTo(new[] { AlicePersonId, DavePersonId });
    }

    [Fact]
    public async Task Subchart_cross_tenant_returns_404()
    {
        // Persons exist in TenantId; viewer is targeting from
        // OtherTenantId — even self-id would 404 because the tenant
        // scopes both the visibility CTE and the subchart CTE.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, OtherTenantId, defaultCallerPersonId: CarolPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync($"/v1/subchart/{BobPersonId:D}").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.NotFound);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:person_not_found");
    }

    [Fact]
    public async Task Subchart_invalid_depth_returns_400()
    {
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: CarolPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync($"/v1/subchart/{CarolPersonId:D}?depth=-1").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:invalid_depth");
    }

    // ── valid_at temporal parameter (#582) ──────────────────────────────

    [Fact]
    public async Task Subchart_at_past_date_returns_historical_parent_and_observation()
    {
        // SCD2: Dave reported to Bob until 2026-03-01, then to a new
        // manager Erin from 2026-03-01 onward. The fixture's current
        // edge Dave→Bob (added by InitializeAsync) is closed off at
        // that boundary and a new edge Dave→Erin opens. valid_at picks
        // the moment, so the same query yields Bob OR Erin.
        //
        // Dave's job_title also moves: "Junior" historically (created
        // pre-T2), "Senior" current (created post-T2). The temporal
        // filter on `persons.created_at` in latest_obs is exercised
        // here — past query must pick the historical title.
        var erin = Guid.Parse("88888888-8888-8888-8888-888888888888");
        var t1   = new DateTime(2026, 2, 1, 0, 0, 0, DateTimeKind.Utc);
        var t2   = new DateTime(2026, 3, 1, 0, 0, 0, DateTimeKind.Utc);
        await SeedPersonAsync(erin, "erin@example.com", "Erin Park").ConfigureAwait(false);
        await InsertObservationHistoricalAsync(DavePersonId, "job_title", "Junior Engineer",
            new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc)).ConfigureAwait(false);
        await InsertObservationHistoricalAsync(DavePersonId, "job_title", "Senior Engineer",
            new DateTime(2026, 4, 1, 0, 0, 0, DateTimeKind.Utc)).ConfigureAwait(false);
        await CloseEdgeAsync(child: DavePersonId, parent: BobPersonId, validTo: t2).ConfigureAwait(false);
        await InsertEdgeHistoricalAsync(child: DavePersonId, parent: erin, validFrom: t2, validTo: null).ConfigureAwait(false);

        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: CarolPersonId);
        var client = app.CreateClient();

        // At T1 (Feb 2026) Dave still reports to Bob AND his title is "Junior Engineer".
        var past = await client.GetAsync($"/v1/subchart/{BobPersonId:D}?depth=1&valid_at={t1:O}").ConfigureAwait(false);
        past.StatusCode.Should().Be(HttpStatusCode.OK);
        var pastDoc = await past.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var pastReports = pastDoc.GetProperty("root").GetProperty("subordinates").EnumerateArray().ToArray();
        var pastDave = pastReports.Single(s => s.GetProperty("person_id").GetGuid() == DavePersonId);
        pastDave.GetProperty("job_title").GetString().Should().Be("Junior Engineer");

        // Now Dave is no longer Bob's child AND his title is "Senior Engineer".
        var now = await client.GetAsync($"/v1/subchart/{BobPersonId:D}?depth=1").ConfigureAwait(false);
        now.StatusCode.Should().Be(HttpStatusCode.OK);
        var nowDoc = await now.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var nowChildren = nowDoc.GetProperty("root").GetProperty("subordinates").EnumerateArray()
            .Select(s => s.GetProperty("person_id").GetGuid()).ToArray();
        nowChildren.Should().NotContain(DavePersonId);
    }

    [Fact]
    public async Task Subchart_valid_at_boundary_inclusive_at_valid_from_exclusive_at_valid_to()
    {
        // SCD2 invariants for the temporal predicate (#582 T2):
        //   valid_at == valid_from  → row INCLUDED (closed-left).
        //   valid_at == valid_to    → row EXCLUDED (open-right).
        var t2 = new DateTime(2026, 3, 1, 0, 0, 0, DateTimeKind.Utc);
        var erin = Guid.Parse("8888aaaa-8888-aaaa-8888-aaaa88880001");
        await SeedPersonAsync(erin, "erin-boundary@example.com", "Erin B").ConfigureAwait(false);
        await CloseEdgeAsync(child: DavePersonId, parent: BobPersonId, validTo: t2).ConfigureAwait(false);
        await InsertEdgeHistoricalAsync(child: DavePersonId, parent: erin, validFrom: t2, validTo: null).ConfigureAwait(false);

        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: CarolPersonId);
        var client = app.CreateClient();

        // Query exactly at t2 — old Dave→Bob edge is now CLOSED (valid_to == t2,
        // predicate requires valid_to > t2); new Dave→Erin edge is OPEN
        // (valid_from == t2, predicate requires valid_from <= t2).
        var atBoundary = await client.GetAsync($"/v1/subchart/{BobPersonId:D}?depth=1&valid_at={t2:O}").ConfigureAwait(false);
        atBoundary.StatusCode.Should().Be(HttpStatusCode.OK);
        var boundaryDoc = await atBoundary.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var boundaryChildren = boundaryDoc.GetProperty("root").GetProperty("subordinates").EnumerateArray()
            .Select(s => s.GetProperty("person_id").GetGuid()).ToArray();
        boundaryChildren.Should().NotContain(DavePersonId, "valid_to=t2 means edge ended AT t2, exclusive");

        // One microsecond before t2: old edge still open.
        var justBefore = t2.AddTicks(-1);
        var beforeDoc = (await (await client.GetAsync($"/v1/subchart/{BobPersonId:D}?depth=1&valid_at={justBefore:O}").ConfigureAwait(false))
            .ReadJsonAsync<JsonElement>().ConfigureAwait(false));
        var beforeChildren = beforeDoc.GetProperty("root").GetProperty("subordinates").EnumerateArray()
            .Select(s => s.GetProperty("person_id").GetGuid()).ToArray();
        beforeChildren.Should().Contain(DavePersonId, "valid_at strictly before valid_to means edge still active");
    }

    [Fact]
    public async Task Forest_at_past_date_reflects_past_tree_shape()
    {
        // Two snapshots of the bamboohr forest:
        //   Until T2 — Carol's root row open; Erin not yet in the org.
        //   From T2 — Erin gets her own no-parent row; she becomes a
        //   second root in the forest.
        var erin = Guid.Parse("99999999-9999-9999-9999-999999999999");
        var t1   = new DateTime(2026, 2, 15, 0, 0, 0, DateTimeKind.Utc);
        var t2   = new DateTime(2026, 3, 15, 0, 0, 0, DateTimeKind.Utc);
        var dirk = Guid.Parse("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        await SeedPersonAsync(erin, "erin2@example.com", "Erin Park").ConfigureAwait(false);
        await SeedPersonAsync(dirk, "dirk@example.com", "Dirk Holt").ConfigureAwait(false);
        await InsertNoParentRowAsync(CarolPersonId).ConfigureAwait(false);
        await InsertEdgeHistoricalAsync(child: dirk, parent: erin, validFrom: t2, validTo: null).ConfigureAwait(false);
        await InsertNoParentRowHistoricalAsync(erin, validFrom: t2).ConfigureAwait(false);
        await InsertVisibilityAsync(viewer: CarolPersonId, viewed: null).ConfigureAwait(false);

        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: CarolPersonId);
        var client = app.CreateClient();

        // Past (T1): only Carol's tree visible — Erin hasn't joined yet.
        var past = await client.GetAsync($"/v1/subchart?valid_at={t1:O}").ConfigureAwait(false);
        past.StatusCode.Should().Be(HttpStatusCode.OK);
        var pastRoots = (await past.ReadJsonAsync<JsonElement>().ConfigureAwait(false))
            .GetProperty("roots").EnumerateArray()
            .Select(r => r.GetProperty("person_id").GetGuid()).ToArray();
        pastRoots.Should().BeEquivalentTo(new[] { CarolPersonId });

        // Now: Carol + Erin both surface.
        var now = await client.GetAsync("/v1/subchart").ConfigureAwait(false);
        now.StatusCode.Should().Be(HttpStatusCode.OK);
        var nowRoots = (await now.ReadJsonAsync<JsonElement>().ConfigureAwait(false))
            .GetProperty("roots").EnumerateArray()
            .Select(r => r.GetProperty("person_id").GetGuid()).ToArray();
        nowRoots.Should().BeEquivalentTo(new[] { CarolPersonId, erin });
    }

    [Fact]
    public async Task Subchart_future_valid_at_returns_400()
    {
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: BobPersonId);
        var client = app.CreateClient();
        var future = DateTime.UtcNow.AddDays(1).ToString("O");

        var response = await client.GetAsync($"/v1/subchart/{BobPersonId:D}?valid_at={future}").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:invalid_valid_at");
    }

    [Fact]
    public async Task Forest_future_valid_at_returns_400()
    {
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: BobPersonId);
        var client = app.CreateClient();
        var future = DateTime.UtcNow.AddDays(1).ToString("O");

        var response = await client.GetAsync($"/v1/subchart?valid_at={future}").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:invalid_valid_at");
    }

    private async Task InsertObservationHistoricalAsync(Guid personId, string valueType, string value, DateTime createdAt)
    {
        // Helper for #582 T1: seed a `persons` observation with an
        // explicit historical `created_at`, so the temporal filter
        // `p.created_at <= COALESCE(@valid_at, UTC_TIMESTAMP(6))` in
        // latest_obs gets exercised by an assertion-level test.
        var col = valueType switch
        {
            "email" or "id" or "username" => "value_id",
            "display_name" or "job_title" or "status" or "department" => "value_full_text",
            _ => "value",
        };
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        var sql = $"""
            INSERT INTO persons
                (value_type, insight_source_type, insight_source_id, insight_tenant_id,
                 {col},
                 person_id, author_person_id, reason, created_at)
            VALUES
                (@vt, 'bamboohr', @src, @tenant,
                 @val,
                 @person, @author, '', @createdAt)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@vt",        valueType);
        cmd.Parameters.AddWithValue("@src",       BambooSourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant",    TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@val",       value);
        cmd.Parameters.AddWithValue("@person",    personId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@author",    AuthorPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@createdAt", createdAt);
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    private async Task CloseEdgeAsync(Guid child, Guid parent, DateTime validTo)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            UPDATE org_chart SET valid_to = @vto
            WHERE insight_tenant_id = @t AND insight_source_type = 'bamboohr'
              AND child_person_id = @c AND parent_person_id = @p AND valid_to IS NULL
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@t",   TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@c",   child.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@p",   parent.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@vto", validTo);
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    private async Task InsertEdgeHistoricalAsync(Guid child, Guid parent, DateTime validFrom, DateTime? validTo)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            INSERT INTO org_chart
                (insight_tenant_id, insight_source_type, insight_source_id,
                 child_person_id, parent_person_id, author_person_id, reason,
                 valid_from, valid_to)
            VALUES (@t, 'bamboohr', @sid, @c, @p, @a, '', @vf, @vto)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@t",   TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@sid", BambooSourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@c",   child.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@p",   parent.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@a",   AuthorPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@vf",  validFrom);
        cmd.Parameters.AddWithValue("@vto", (object?)validTo ?? DBNull.Value);
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    private async Task InsertNoParentRowHistoricalAsync(Guid child, DateTime validFrom)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            INSERT INTO org_chart
                (insight_tenant_id, insight_source_type, insight_source_id,
                 child_person_id, parent_person_id, author_person_id, reason,
                 valid_from, valid_to)
            VALUES (@t, 'bamboohr', @sid, @c, NULL, @a, '', @vf, NULL)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@t",   TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@sid", BambooSourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@c",   child.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@a",   AuthorPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@vf",  validFrom);
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    // ── Forest variant: GET /v1/subchart (no person_id) (#344) ──────────

    [Fact]
    public async Task Forest_caller_with_only_self_returns_their_own_tree()
    {
        // Bob has no visibility grants — his visible_set is himself plus
        // his org_chart descendants {Bob, Alice, Dave}. Carol (his parent)
        // is invisible, so Bob is a root in his own view. Bob has direct
        // reports → not an orphan → forest = 1 tree.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: BobPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync("/v1/subchart").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var roots = doc.GetProperty("roots").EnumerateArray().ToArray();
        roots.Should().ContainSingle();
        roots[0].GetProperty("person_id").GetGuid().Should().Be(BobPersonId);
        roots[0].GetProperty("subordinates").EnumerateArray()
            .Select(s => s.GetProperty("person_id").GetGuid())
            .Should().BeEquivalentTo(new[] { AlicePersonId, DavePersonId });
    }

    [Fact]
    public async Task Forest_caller_with_wildcard_visibility_returns_real_tops_only()
    {
        // Outsider holds a wildcard grant (viewed_person_id IS NULL) so
        // visible_set expands to every person in the tenant. Tops of the
        // bamboohr forest = Carol (parent NULL — added via path-B). The
        // outsider's own no-parent row + Alice's + Dave's are orphan and
        // filtered. Result: 1 tree rooted at Carol.
        await InsertNoParentRowAsync(CarolPersonId).ConfigureAwait(false);
        await InsertNoParentRowAsync(OutsiderPersonId).ConfigureAwait(false);
        await InsertVisibilityAsync(viewer: OutsiderPersonId, viewed: null).ConfigureAwait(false);
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync("/v1/subchart").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var roots = doc.GetProperty("roots").EnumerateArray().ToArray();
        roots.Should().ContainSingle();
        roots[0].GetProperty("person_id").GetGuid().Should().Be(CarolPersonId);
        // Whole tree under Carol is reachable.
        var lvl1 = roots[0].GetProperty("subordinates").EnumerateArray().ToArray();
        lvl1.Should().ContainSingle().Which.GetProperty("person_id").GetGuid().Should().Be(BobPersonId);
    }

    [Fact]
    public async Task Forest_caller_with_peer_grant_returns_both_trees()
    {
        // Two parallel trees: existing Carol→Bob→{Alice,Dave} and a
        // freshly added Eve→Frank. Bob holds a grant on Eve. Bob sees
        // his own subtree (he's invisible to himself's parent) plus
        // Eve's subtree → 2 roots. Eve gets a no-parent row (what
        // path-B rebuild would write) so she's in_source as a top.
        var eve   = Guid.Parse("66666666-6666-6666-6666-666666666666");
        var frank = Guid.Parse("77777777-7777-7777-7777-777777777777");
        await SeedPersonAsync(eve,   "eve@example.com",   "Eve Stone").ConfigureAwait(false);
        await SeedPersonAsync(frank, "frank@example.com", "Frank Holt").ConfigureAwait(false);
        await InsertNoParentRowAsync(eve).ConfigureAwait(false);
        await InsertEdgeAsync(child: frank, parent: eve).ConfigureAwait(false);
        await InsertVisibilityAsync(viewer: BobPersonId, viewed: eve).ConfigureAwait(false);

        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: BobPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync("/v1/subchart").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var rootIds = doc.GetProperty("roots").EnumerateArray()
            .Select(r => r.GetProperty("person_id").GetGuid()).ToArray();
        rootIds.Should().BeEquivalentTo(new[] { BobPersonId, eve });
    }

    [Fact]
    public async Task Forest_caller_with_grant_on_manager_returns_one_tree_from_manager()
    {
        // Alice gets a grant on Carol — visible_set = {Alice, Carol,
        // Bob, Dave} (Carol's descendants close the set). Carol gets a
        // no-parent row (what path-B rebuild would write for a top) so
        // she's in_source; she becomes the single root and Alice falls
        // inside her tree → 1 tree.
        await InsertNoParentRowAsync(CarolPersonId).ConfigureAwait(false);
        await InsertVisibilityAsync(viewer: AlicePersonId, viewed: CarolPersonId).ConfigureAwait(false);
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: AlicePersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync("/v1/subchart").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var roots = doc.GetProperty("roots").EnumerateArray().ToArray();
        roots.Should().ContainSingle();
        roots[0].GetProperty("person_id").GetGuid().Should().Be(CarolPersonId);
    }

    [Fact]
    public async Task Forest_caller_not_in_any_source_returns_empty_roots()
    {
        // Outsider has no org_chart row and no visibility grants. The
        // endpoint stays a 200 — empty roots, never 404, per team
        // decision (visibility absence is data fact, not error).
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync("/v1/subchart").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("roots").GetArrayLength().Should().Be(0);
    }

    [Fact]
    public async Task Forest_orphan_singleton_root_is_filtered()
    {
        // Outsider gets a no-parent row + a wildcard grant. He's now a
        // visible root with no children → the orphan filter (#344
        // decision) drops him from the response. Carol still shows.
        await InsertNoParentRowAsync(CarolPersonId).ConfigureAwait(false);
        await InsertNoParentRowAsync(OutsiderPersonId).ConfigureAwait(false);
        await InsertVisibilityAsync(viewer: OutsiderPersonId, viewed: null).ConfigureAwait(false);
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync("/v1/subchart").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var roots = doc.GetProperty("roots").EnumerateArray()
            .Select(r => r.GetProperty("person_id").GetGuid()).ToArray();
        roots.Should().BeEquivalentTo(new[] { CarolPersonId });
        roots.Should().NotContain(OutsiderPersonId);
    }

    [Fact]
    public async Task Forest_depth_zero_returns_tops_only_no_subordinates()
    {
        // Wildcard caller with depth=0 sees the real top(s) — Carol —
        // but no descent. Orphan filter does NOT drop her because the
        // SQL-side EXISTS check is on org_chart, not on the depth-
        // limited subtree (#344).
        await InsertNoParentRowAsync(CarolPersonId).ConfigureAwait(false);
        await InsertVisibilityAsync(viewer: OutsiderPersonId, viewed: null).ConfigureAwait(false);
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync("/v1/subchart?depth=0").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        var roots = doc.GetProperty("roots").EnumerateArray().ToArray();
        roots.Should().ContainSingle();
        roots[0].GetProperty("person_id").GetGuid().Should().Be(CarolPersonId);
        roots[0].GetProperty("subordinates").GetArrayLength().Should().Be(0);
    }

    [Fact]
    public async Task Forest_no_caller_returns_401()
    {
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: null);
        var client = app.CreateClient();

        var response = await client.GetAsync("/v1/subchart").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:caller_unresolved");
    }

    [Fact]
    public async Task Forest_invalid_depth_returns_400()
    {
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: BobPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync("/v1/subchart?depth=-1").ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:invalid_depth");
    }

    private async Task InsertNoParentRowAsync(Guid child)
    {
        // Parent-less row helper (#579): same shape as InsertEdgeAsync
        // but with parent_person_id = NULL. Mirrors what
        // SqlPersonsSeed.InsertOrgChartForTenant writes for tops and
        // singletons. valid_from anchored at 2020-01-01 so temporal
        // tests can pick valid_at in the past and still see the row.
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            INSERT INTO org_chart
                (insight_tenant_id, insight_source_type, insight_source_id,
                 child_person_id, parent_person_id, author_person_id, reason,
                 valid_from, valid_to)
            VALUES (@t, 'bamboohr', @sid, @c, NULL, @a, '', '2020-01-01 00:00:00', NULL)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@t",   TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@sid", BambooSourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@c",   child.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@a",   AuthorPersonId.ToByteArray(bigEndian: true));
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    // ── Helpers (mirror VisibilityGateTests so the fixture stays small) ─

    private async Task SeedPersonAsync(Guid personId, string email, string displayName)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        await InsertObservationAsync(conn, personId, "email",        email).ConfigureAwait(false);
        await InsertObservationAsync(conn, personId, "display_name", displayName).ConfigureAwait(false);
    }

    private static async Task InsertObservationAsync(
        MySqlConnection conn, Guid personId, string valueType, string value)
    {
        var col = valueType switch
        {
            "email" or "id" or "username" => "value_id",
            "display_name" => "value_full_text",
            _ => "value",
        };
        var sql = $"""
            INSERT IGNORE INTO persons
                (value_type, insight_source_type, insight_source_id, insight_tenant_id,
                 {col},
                 person_id, author_person_id, reason, created_at)
            VALUES
                (@vt, 'bamboohr', @src, @tenant,
                 @val,
                 @person, @author, '', UTC_TIMESTAMP(6))
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@vt",     valueType);
        cmd.Parameters.AddWithValue("@src",    BambooSourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant", TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@val",    value);
        cmd.Parameters.AddWithValue("@person", personId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@author", AuthorPersonId.ToByteArray(bigEndian: true));
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    private async Task InsertEdgeAsync(Guid child, Guid parent)
    {
        // valid_from anchored at 2020-01-01 so the row is active under
        // any reasonable past valid_at value tests pick (#582 temporal
        // scenarios). The temporal predicate at the SQL layer still
        // includes the row at "now" because valid_to IS NULL.
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            INSERT INTO org_chart
                (insight_tenant_id, insight_source_type, insight_source_id,
                 child_person_id, parent_person_id, author_person_id, reason,
                 valid_from, valid_to)
            VALUES (@t, 'bamboohr', @sid, @c, @p, @a, '', '2020-01-01 00:00:00', NULL)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@t",   TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@sid", BambooSourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@c",   child.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@p",   parent.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@a",   AuthorPersonId.ToByteArray(bigEndian: true));
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    private async Task InsertVisibilityAsync(Guid viewer, Guid? viewed, DateTime? validTo = null)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            INSERT INTO visibility
                (visibility_id, insight_tenant_id, viewer_person_id, viewed_person_id,
                 valid_from, valid_to, author_person_id, reason)
            VALUES (@id, @tenant, @viewer, @viewed, '2020-01-01 00:00:00', @valid_to, @viewer, NULL)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@id",       Guid.NewGuid().ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant",   TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@viewer",   viewer.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@viewed",   viewed is { } v ? v.ToByteArray(bigEndian: true) : (object)DBNull.Value);
        cmd.Parameters.AddWithValue("@valid_to", validTo is { } t ? t : (object)DBNull.Value);
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }
}
