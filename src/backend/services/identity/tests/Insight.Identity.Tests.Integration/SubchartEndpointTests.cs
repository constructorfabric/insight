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
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            INSERT INTO org_chart
                (insight_tenant_id, insight_source_type, insight_source_id,
                 child_person_id, parent_person_id, author_person_id, reason,
                 valid_from, valid_to)
            VALUES (@t, 'bamboohr', @sid, @c, @p, @a, '', UTC_TIMESTAMP(6), NULL)
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
