using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using FluentAssertions;
using Insight.Identity.Api.Contracts;
using Insight.Identity.Api.Auth;
using MySqlConnector;
using Xunit;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// End-to-end tests for <c>POST /v1/profiles</c>. Each test seeds rows
/// directly into the Testcontainers MariaDB and hits the endpoint via
/// <see cref="TestApplicationFactory"/>.
/// </summary>
[Collection(MariaDbCollection.Name)]
public sealed class ProfilesEndpointTests : IAsyncLifetime
{
    private static readonly Guid TenantId        = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid BambooSourceId  = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    private static readonly Guid SlackSourceId   = Guid.Parse("dddddddd-dddd-dddd-dddd-dddddddddddd");
    private static readonly Guid AlicePersonId   = Guid.Parse("cccccccc-cccc-cccc-cccc-cccccccccccc");
    private static readonly Guid SecondPersonId  = Guid.Parse("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee");
    private static readonly Guid CallerPersonId  = Guid.Parse("ddddddd0-0000-0000-0000-000000000003");
    private static readonly Guid AuthorPersonId  = Guid.Empty;

    private readonly MariaDbFixture _fixture;
    private TestApplicationFactory? _app;

    public ProfilesEndpointTests(MariaDbFixture fixture) => _fixture = fixture;

    public async Task InitializeAsync()
    {
        await _fixture.ResetAsync().ConfigureAwait(false);
        _app = new TestApplicationFactory(_fixture.ConnectionString, TenantId, defaultCallerPersonId: CallerPersonId);
        await _fixture.SeedWholeTenantVisibilityAsync(TenantId, CallerPersonId).ConfigureAwait(false);
    }

    public Task DisposeAsync()
    {
        _app?.Dispose();
        return Task.CompletedTask;
    }

    // ── 200 OK — email lookup ─────────────────────────────────────────

    [Fact]
    public async Task Email_lookup_returns_profile_with_ids_list()
    {
        await SeedAliceAsync().ConfigureAwait(false);
        var client = _app!.CreateClient();

        var body = new ResolveProfileCommandModel("email", "alice@example.com", null, null);
        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        await ShouldSucceedAsync(response).ConfigureAwait(false);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);

        doc.GetProperty("person_id").GetGuid().Should().Be(AlicePersonId);
        doc.GetProperty("insight_tenant_id").GetGuid().Should().Be(TenantId);
        doc.GetProperty("email").GetString().Should().Be("alice@example.com");
        doc.GetProperty("display_name").GetString().Should().Be("Alice Smith");
        doc.GetProperty("job_title").GetString().Should().Be("Staff Engineer");

        var ids = doc.GetProperty("ids").EnumerateArray().ToArray();
        ids.Should().HaveCount(2);
        ids.Should().Contain(e => e.GetProperty("insight_source_type").GetString() == "bamboohr"
                               && e.GetProperty("value").GetString() == "alice-bamboo-001");
        ids.Should().Contain(e => e.GetProperty("insight_source_type").GetString() == "slack"
                               && e.GetProperty("value").GetString() == "U03ABCDEF");
    }

    [Fact]
    public async Task Email_lookup_lowercases_input()
    {
        await SeedAliceAsync().ConfigureAwait(false);
        var client = _app!.CreateClient();

        var body = new ResolveProfileCommandModel("email", "Alice@Example.COM", null, null);
        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
    }

    // ── 200 OK — id lookup ────────────────────────────────────────────

    [Fact]
    public async Task Id_lookup_within_source_returns_profile()
    {
        await SeedAliceAsync().ConfigureAwait(false);
        var client = _app!.CreateClient();

        var body = new ResolveProfileCommandModel(
            ValueType: "id",
            Value: "alice-bamboo-001",
            InsightSourceType: "bamboohr",
            InsightSourceId: BambooSourceId);

        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        await ShouldSucceedAsync(response).ConfigureAwait(false);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);

        doc.GetProperty("person_id").GetGuid().Should().Be(AlicePersonId);
    }

    // ── 404 paths ─────────────────────────────────────────────────────

    [Fact]
    public async Task Returns_404_when_email_unknown()
    {
        var client = _app!.CreateClient();
        var body = new ResolveProfileCommandModel("email", "nobody@example.com", null, null);
        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.NotFound);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:person_not_found");
    }

    [Fact]
    public async Task Returns_404_when_id_unknown_in_source()
    {
        await SeedAliceAsync().ConfigureAwait(false);
        var client = _app!.CreateClient();
        var body = new ResolveProfileCommandModel(
            "id", "no-such-id", "bamboohr", BambooSourceId);
        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    [Fact]
    public async Task Returns_404_for_rebound_email_old_value()
    {
        // Alice's email was alice@example.com initially, then rebound to
        // alice@new.example.com on the same (bamboohr, source_id). Per
        // ADR-0003 latest-per-source semantics, the old email no longer
        // resolves — there is no current observation with value_id =
        // 'alice@example.com' that wins rn=1 on its partition.
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        await InsertAsync(conn, "email", AlicePersonId, "alice@example.com", isValueId: true,
            createdAtSqlExpr: "DATE_SUB(UTC_TIMESTAMP(6), INTERVAL 30 DAY)").ConfigureAwait(false);
        await InsertAsync(conn, "email", AlicePersonId, "alice@new.example.com", isValueId: true).ConfigureAwait(false);

        var client = _app!.CreateClient();
        var body = new ResolveProfileCommandModel("email", "alice@example.com", null, null);
        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    // ── 422 ambiguous ────────────────────────────────────────────────

    [Fact]
    public async Task Returns_422_when_two_persons_share_current_email()
    {
        // Data-invariant violation: two different person_ids both have
        // a current 'email' observation with the same value_id within
        // the same tenant. The service must NOT pick one; it must
        // surface the ambiguity.
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        await InsertAsync(conn, "email", AlicePersonId,  "shared@example.com", isValueId: true).ConfigureAwait(false);
        await InsertAsync(conn, "email", SecondPersonId, "shared@example.com", isValueId: true,
            insightSourceIdOverride: SlackSourceId, sourceType: "slack").ConfigureAwait(false);

        var client = _app!.CreateClient();
        var body = new ResolveProfileCommandModel("email", "shared@example.com", null, null);
        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.UnprocessableEntity);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:ambiguous_profile");
        var personIds = doc.GetProperty("person_ids").EnumerateArray().Select(e => e.GetGuid()).ToArray();
        personIds.Should().BeEquivalentTo([AlicePersonId, SecondPersonId]);
    }

    // ── 400 validation paths ─────────────────────────────────────────

    [Fact]
    public async Task Returns_400_when_value_type_missing()
    {
        var client = _app!.CreateClient();
        var body = new ResolveProfileCommandModel(null, "x", null, null);
        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:invalid_value_type");
    }

    [Fact]
    public async Task Returns_400_when_id_lookup_missing_source_fields()
    {
        var client = _app!.CreateClient();
        var body = new ResolveProfileCommandModel("id", "12345", null, null);
        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:missing_source_for_id");
    }

    [Fact]
    public async Task Returns_400_when_email_lookup_includes_source_fields()
    {
        var client = _app!.CreateClient();
        var body = new ResolveProfileCommandModel("email", "alice@x.com", "bamboohr", BambooSourceId);
        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:source_not_allowed_for_email");
    }

    [Fact]
    public async Task Returns_400_when_no_tenant_resolved()
    {
        using var noTenantApp = new TestApplicationFactory(
            _fixture.ConnectionString, defaultTenantId: null, defaultCallerPersonId: CallerPersonId);
        var client = noTenantApp.CreateClient();

        var body = new ResolveProfileCommandModel("email", "alice@x.com", null, null);
        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:tenant_unresolved");
    }

    [Fact]
    public async Task Returns_401_when_no_caller_resolved()
    {
        using var noCallerApp = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: null);
        var client = noCallerApp.CreateClient();

        var body = new ResolveProfileCommandModel("email", "alice@x.com", null, null);
        var response = await client.PostJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:caller_unresolved");
    }

    // ── Seed helpers ─────────────────────────────────────────────────

    private async Task SeedAliceAsync()
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);

        // BambooHR observations for Alice
        await InsertAsync(conn, "email",        AlicePersonId, "alice@example.com",  isValueId: true).ConfigureAwait(false);
        await InsertAsync(conn, "id",           AlicePersonId, "alice-bamboo-001",   isValueId: true).ConfigureAwait(false);
        await InsertAsync(conn, "display_name", AlicePersonId, "Alice Smith",        isFullText: true).ConfigureAwait(false);
        await InsertAsync(conn, "first_name",   AlicePersonId, "Alice",              isFullText: true).ConfigureAwait(false);
        await InsertAsync(conn, "last_name",    AlicePersonId, "Smith",              isFullText: true).ConfigureAwait(false);
        await InsertAsync(conn, "job_title",    AlicePersonId, "Staff Engineer",     isFullText: true).ConfigureAwait(false);
        await InsertAsync(conn, "department",   AlicePersonId, "Engineering",        isFullText: true).ConfigureAwait(false);

        // Slack source — second 'id' binding for the same person
        await InsertAsync(conn, "id", AlicePersonId, "U03ABCDEF", isValueId: true,
            insightSourceIdOverride: SlackSourceId, sourceType: "slack").ConfigureAwait(false);
    }

    private static async Task InsertAsync(
        MySqlConnection conn,
        string valueType,
        Guid personId,
        string value,
        bool isValueId = false,
        bool isFullText = false,
        Guid? insightSourceIdOverride = null,
        string sourceType = "bamboohr",
        string? createdAtSqlExpr = null)
    {
        var createdAtClause = createdAtSqlExpr ?? "UTC_TIMESTAMP(6)";
        var sql = $$"""
            INSERT INTO persons
                (value_type, insight_source_type, insight_source_id, insight_tenant_id,
                 value_id, value_full_text, value,
                 person_id, author_person_id, reason, created_at)
            VALUES
                (@vt, @st, @src, @tenant,
                 @vid, @vft, @vraw,
                 @person, @author, '', {{createdAtClause}})
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@vt", valueType);
        cmd.Parameters.AddWithValue("@st", sourceType);
        cmd.Parameters.AddWithValue("@src", (insightSourceIdOverride ?? BambooSourceId).ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant", TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@vid", isValueId ? value : DBNull.Value);
        cmd.Parameters.AddWithValue("@vft", isFullText ? value : DBNull.Value);
        cmd.Parameters.AddWithValue("@vraw", (!isValueId && !isFullText) ? value : DBNull.Value);
        cmd.Parameters.AddWithValue("@person", personId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@author", AuthorPersonId.ToByteArray(bigEndian: true));
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    private static async Task ShouldSucceedAsync(HttpResponseMessage response)
    {
        if (!response.IsSuccessStatusCode)
        {
            var body = await response.Content.ReadAsStringAsync().ConfigureAwait(false);
            throw new InvalidOperationException($"Expected 2xx, got {(int)response.StatusCode}. Body: {body}");
        }
    }
}
