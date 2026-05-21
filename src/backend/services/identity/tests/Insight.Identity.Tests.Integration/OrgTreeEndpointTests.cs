using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using FluentAssertions;
using Insight.Identity.Api.Contracts;
using MySqlConnector;
using Xunit;

namespace Insight.Identity.Tests.Integration;

[Collection(MariaDbCollection.Name)]
public sealed class OrgTreeEndpointTests : IAsyncLifetime
{
    private static readonly Guid TenantId       = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid BambooSourceId = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    private static readonly Guid SlackSourceId  = Guid.Parse("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee");
    private static readonly Guid CarolPersonId  = Guid.Parse("11111111-1111-1111-1111-111111111111");
    private static readonly Guid BobPersonId    = Guid.Parse("22222222-2222-2222-2222-222222222222");
    private static readonly Guid AlicePersonId  = Guid.Parse("33333333-3333-3333-3333-333333333333");
    private static readonly Guid DavePersonId   = Guid.Parse("44444444-4444-4444-4444-444444444444");
    private static readonly Guid AuthorPersonId = Guid.Empty;
    private static readonly string[] BobReportEmails = { "alice@example.com", "dave@example.com" };

    private readonly MariaDbFixture _fixture;
    private TestApplicationFactory? _app;

    public OrgTreeEndpointTests(MariaDbFixture fixture) => _fixture = fixture;

    public async Task InitializeAsync()
    {
        await _fixture.ResetAsync().ConfigureAwait(false);
        _app = new TestApplicationFactory(_fixture.ConnectionString, TenantId);
        await SeedTreeAsync().ConfigureAwait(false);
    }

    public Task DisposeAsync()
    {
        _app?.Dispose();
        return Task.CompletedTask;
    }

    [Fact]
    public async Task GET_persons_alice_returns_supervisor_pair_from_org_chart()
    {
        var client = _app!.CreateClient();
        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.OK);

        // RFC 8594 — endpoint deprecated; every response carries the
        // signalling headers steering callers to POST /v1/profiles.
        response.Headers.GetValues("Deprecation").Should().ContainSingle().Which.Should().Be("true");
        response.Headers.GetValues("Link").Should().ContainSingle()
            .Which.Should().Contain("/v1/profiles").And.Contain("rel=\"successor-version\"");

        var doc = await response.Content.ReadFromJsonAsync<JsonElement>().ConfigureAwait(false);

        doc.GetProperty("person_id").GetGuid().Should().Be(AlicePersonId);
        doc.GetProperty("email").GetString().Should().Be("alice@example.com");
        doc.GetProperty("display_name").GetString().Should().Be("Alice Smith");
        doc.GetProperty("job_title").GetString().Should().Be("Staff Engineer");
        doc.GetProperty("status").GetString().Should().Be("Active");

        // Supervisor pair — hydrated from BambooHR org_chart edge.
        doc.GetProperty("supervisor_email").GetString().Should().Be("bob@example.com");
        doc.GetProperty("supervisor_name").GetString().Should().Be("Jones, Bob");

        // Legacy parent_* triple mirrors the same edge.
        doc.GetProperty("parent_email").GetString().Should().Be("bob@example.com");
        doc.GetProperty("parent_id").GetString().Should().Be("BOB-7");
        doc.GetProperty("parent_person_id").GetGuid().Should().Be(BobPersonId);

        // Alice is a leaf — no subordinates.
        doc.GetProperty("subordinates").EnumerateArray().Should().BeEmpty();
    }

    [Fact]
    public async Task GET_persons_bob_returns_recursive_subordinates_from_org_chart()
    {
        var client = _app!.CreateClient();
        var response = await client.GetAsync(new Uri("/v1/persons/bob@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.OK);

        var doc = await response.Content.ReadFromJsonAsync<JsonElement>().ConfigureAwait(false);

        doc.GetProperty("email").GetString().Should().Be("bob@example.com");

        var subs = doc.GetProperty("subordinates").EnumerateArray().ToArray();
        subs.Should().HaveCount(2);
        var subEmails = subs.Select(s => s.GetProperty("email").GetString()).ToArray();
        subEmails.Should().BeEquivalentTo(BobReportEmails);

        // Recursion shape: each subordinate is itself a full PersonResponse.
        foreach (var sub in subs)
        {
            sub.GetProperty("display_name").GetString().Should().NotBeNullOrEmpty();
            sub.GetProperty("supervisor_email").GetString().Should().Be("bob@example.com");
            sub.GetProperty("subordinates").EnumerateArray().Should().BeEmpty();
        }
    }

    [Fact]
    public async Task GET_persons_alice_does_not_surface_slack_parent_edge()
    {
        // Alice has a Slack edge (Carol) in addition to her BambooHR
        // edge (Bob). Only BambooHR drives the response.
        await InsertEdgeAsync("slack", SlackSourceId, child: AlicePersonId, parent: CarolPersonId).ConfigureAwait(false);

        var client = _app!.CreateClient();
        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.OK);

        var doc = await response.Content.ReadFromJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("parent_person_id").GetGuid().Should().Be(BobPersonId);
        doc.GetProperty("supervisor_email").GetString().Should().Be("bob@example.com");
    }

    [Fact]
    public async Task GET_persons_bob_with_expand_subordinates_false_keeps_supervisor_drops_subtree()
    {
        // Kill-switch: subordinates recursion is gated by
        // identity.expand_subordinates. Parent hydration is unconditional,
        // so supervisor_email / parent_* must still surface.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString,
            TenantId,
            expandSubordinates: false);
        var client = app.CreateClient();
        var response = await client.GetAsync(new Uri("/v1/persons/bob@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.OK);

        var doc = await response.Content.ReadFromJsonAsync<JsonElement>().ConfigureAwait(false);

        doc.GetProperty("email").GetString().Should().Be("bob@example.com");

        // Parent pair still hydrated — Bob reports to Carol via BambooHR.
        doc.GetProperty("supervisor_email").GetString().Should().Be("carol@example.com");
        doc.GetProperty("parent_person_id").GetGuid().Should().Be(CarolPersonId);

        // Subtree suppressed — kill-switch wins regardless of seed.
        doc.GetProperty("subordinates").EnumerateArray().Should().BeEmpty();
    }

    [Fact]
    public async Task POST_profiles_email_returns_same_org_tree_plus_ids_list()
    {
        var client = _app!.CreateClient();
        var body = new ResolveProfileCommandModel("email", "alice@example.com", null, null);

        var response = await client.PostAsJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.OK);

        var doc = await response.Content.ReadFromJsonAsync<JsonElement>().ConfigureAwait(false);

        // Profile-specific fields.
        doc.GetProperty("person_id").GetGuid().Should().Be(AlicePersonId);
        doc.GetProperty("insight_tenant_id").GetGuid().Should().Be(TenantId);

        // Same org-tree projection as /v1/persons.
        doc.GetProperty("supervisor_email").GetString().Should().Be("bob@example.com");
        doc.GetProperty("supervisor_name").GetString().Should().Be("Jones, Bob");
        doc.GetProperty("parent_email").GetString().Should().Be("bob@example.com");
        doc.GetProperty("parent_id").GetString().Should().Be("BOB-7");
        doc.GetProperty("parent_person_id").GetGuid().Should().Be(BobPersonId);

        // ids[] — full list of source-native id bindings.
        var ids = doc.GetProperty("ids").EnumerateArray().ToArray();
        ids.Should().Contain(e =>
            e.GetProperty("insight_source_type").GetString() == "bamboohr"
            && e.GetProperty("value").GetString() == "ALICE-1");
    }

    [Fact]
    public async Task POST_profiles_root_has_no_supervisor_and_subtree_under_it()
    {
        var client = _app!.CreateClient();
        var body = new ResolveProfileCommandModel("email", "carol@example.com", null, null);

        var response = await client.PostAsJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.OK);

        var doc = await response.Content.ReadFromJsonAsync<JsonElement>().ConfigureAwait(false);

        // Carol is the root — no supervisor.
        // Null-condition serialisation drops the property entirely for
        // ProfileResponse, so probe via TryGetProperty.
        doc.TryGetProperty("supervisor_email", out _).Should().BeFalse();
        doc.TryGetProperty("supervisor_name", out _).Should().BeFalse();
        doc.TryGetProperty("parent_person_id", out _).Should().BeFalse();

        // Subordinates: Carol → Bob → (Alice, Dave). Bob is Carol's only
        // direct report; Alice + Dave hang off Bob.
        var subs = doc.GetProperty("subordinates").EnumerateArray().ToArray();
        subs.Should().HaveCount(1);
        var bob = subs[0];
        bob.GetProperty("email").GetString().Should().Be("bob@example.com");
        var bobSubs = bob.GetProperty("subordinates").EnumerateArray().ToArray();
        bobSubs.Select(s => s.GetProperty("email").GetString())
            .Should().BeEquivalentTo(BobReportEmails);
    }

    // ── Tree seed: Carol (root) → Bob → (Alice, Dave). All BambooHR. ──

    private async Task SeedTreeAsync()
    {
        await SeedPersonAsync(CarolPersonId, "carol@example.com", "Carol Lee", "VP Engineering", "CAROL-1").ConfigureAwait(false);
        await SeedPersonAsync(BobPersonId,   "bob@example.com",   "Jones, Bob", "Engineering Manager", "BOB-7").ConfigureAwait(false);
        await SeedPersonAsync(AlicePersonId, "alice@example.com", "Alice Smith", "Staff Engineer", "ALICE-1").ConfigureAwait(false);
        await SeedPersonAsync(DavePersonId,  "dave@example.com",  "Dave Ng", "Senior Engineer", "DAVE-3").ConfigureAwait(false);

        await InsertEdgeAsync("bamboohr", BambooSourceId, child: BobPersonId,   parent: CarolPersonId).ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: DavePersonId,  parent: BobPersonId).ConfigureAwait(false);
    }

    private async Task SeedPersonAsync(
        Guid personId,
        string email,
        string displayName,
        string jobTitle,
        string sourceNativeId)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);

        await InsertObservationAsync(conn, personId, "email",        email);
        await InsertObservationAsync(conn, personId, "display_name", displayName);
        await InsertObservationAsync(conn, personId, "job_title",    jobTitle);
        await InsertObservationAsync(conn, personId, "status",       "Active");
        await InsertObservationAsync(conn, personId, "id",           sourceNativeId);
    }

    private static async Task InsertObservationAsync(
        MySqlConnection conn,
        Guid personId,
        string valueType,
        string value)
    {
        // ADR-0007: route by value_type so seeded rows match how the
        // ingest pipeline writes them.
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
        cmd.Parameters.AddWithValue("@vt", valueType);
        cmd.Parameters.AddWithValue("@src", BambooSourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant", TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@val", value);
        cmd.Parameters.AddWithValue("@person", personId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@author", AuthorPersonId.ToByteArray(bigEndian: true));
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    private async Task InsertEdgeAsync(
        string sourceType,
        Guid sourceId,
        Guid child,
        Guid parent)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            INSERT INTO org_chart
                (insight_tenant_id, insight_source_type, insight_source_id,
                 child_person_id, parent_person_id, author_person_id, reason,
                 valid_from, valid_to)
            VALUES (@t, @st, @sid, @c, @p, @a, '', UTC_TIMESTAMP(6), NULL)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@t",   TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@st",  sourceType);
        cmd.Parameters.AddWithValue("@sid", sourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@c",   child.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@p",   parent.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@a",   AuthorPersonId.ToByteArray(bigEndian: true));
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }
}
