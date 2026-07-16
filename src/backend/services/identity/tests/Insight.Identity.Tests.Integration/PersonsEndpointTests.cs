using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using FluentAssertions;
using Insight.Identity.Api.Auth;
using MySqlConnector;
using Xunit;

namespace Insight.Identity.Tests.Integration;

[Collection(MariaDbCollection.Name)]
public sealed class PersonsEndpointTests : IAsyncLifetime
{
    private static readonly Guid TenantId         = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid SourceId         = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    private static readonly Guid AlicePersonId    = Guid.Parse("cccccccc-cccc-cccc-cccc-cccccccccccc");
    private static readonly Guid CallerPersonId   = Guid.Parse("ddddddd0-0000-0000-0000-000000000001");
    private static readonly Guid AuthorPersonId   = Guid.Empty;

    private readonly MariaDbFixture _fixture;
    private TestApplicationFactory? _app;

    public PersonsEndpointTests(MariaDbFixture fixture) => _fixture = fixture;

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

    [Fact]
    public async Task Returns_404_when_unknown_email()
    {
        var client = _app!.CreateClient();
        var response = await client.GetAsync(new Uri("/v1/persons/nobody@example.com", UriKind.Relative)).ConfigureAwait(false);
        var body = await response.Content.ReadAsStringAsync().ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.NotFound, "body was: {0}", body);
    }

    [Fact]
    public async Task Returns_person_with_assembled_attributes()
    {
        await SeedAliceAsync().ConfigureAwait(false);
        var client = _app!.CreateClient();

        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        if (!response.IsSuccessStatusCode)
        {
            var body = await response.Content.ReadAsStringAsync().ConfigureAwait(false);
            throw new InvalidOperationException($"Expected 2xx, got {(int)response.StatusCode}. Body: {body}");
        }
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);

        doc.GetProperty("email").GetString().Should().Be("alice@example.com");
        doc.GetProperty("display_name").GetString().Should().Be("Alice Smith");
        doc.GetProperty("first_name").GetString().Should().Be("Alice");
        doc.GetProperty("last_name").GetString().Should().Be("Smith");
        doc.GetProperty("job_title").GetString().Should().Be("Staff Engineer");
        doc.GetProperty("person_id").GetGuid().Should().Be(AlicePersonId);
    }

    [Fact]
    public async Task Returns_400_when_no_tenant_resolved()
    {
        // Caller present (default bearer carries `sub`), no default tenant
        // configured and no `tenant_id` claim on the token → tenant resolver
        // null → 400.
        using var noTenantApp = new TestApplicationFactory(
            _fixture.ConnectionString, defaultTenantId: null, defaultCallerPersonId: CallerPersonId);
        var client = noTenantApp.CreateClient();

        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
    }

    [Fact]
    public async Task Returns_401_when_no_caller_resolved()
    {
        // Caller header missing → endpoint short-circuits with 401
        // before even looking at the tenant.
        using var noCallerApp = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: null);
        var client = noCallerApp.CreateClient();

        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.Unauthorized);
    }

    [Fact]
    public async Task Resolves_tenant_from_jwt_tenant_id_claim()
    {
        // Proves the JwtBearer middleware wires the bearer payload into
        // `HttpContext.User` so `GatewayTenantContext` reads the signed single
        // `tenant_id` claim — no config default, the only path to a tenant
        // is the JWT. Both caller (`sub`) and tenant come from the token.
        await SeedAliceAsync().ConfigureAwait(false);
        using var jwtApp = new TestApplicationFactory(
            _fixture.ConnectionString, defaultTenantId: null, defaultCallerPersonId: CallerPersonId);
        await _fixture.SeedWholeTenantVisibilityAsync(TenantId, CallerPersonId).ConfigureAwait(false);
        var client = jwtApp.CreateClient();
        client.DefaultRequestHeaders.Authorization =
            new System.Net.Http.Headers.AuthenticationHeaderValue(
                "Bearer",
                TestApplicationFactory.BuildJwt(
                    ("sub", CallerPersonId.ToString("D")),
                    ("tenant_id", TenantId.ToString("D"))));

        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        if (!response.IsSuccessStatusCode)
        {
            var body = await response.Content.ReadAsStringAsync().ConfigureAwait(false);
            throw new InvalidOperationException($"Expected 200, got {(int)response.StatusCode}. Body: {body}");
        }
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("email").GetString().Should().Be("alice@example.com");
    }

    private async Task SeedAliceAsync()
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);

        await InsertAsync(conn, "email",        AlicePersonId, "alice@example.com", isValueId: true);
        await InsertAsync(conn, "display_name", AlicePersonId, "Alice Smith",       isValueId: false, isFullText: true);
        await InsertAsync(conn, "job_title",    AlicePersonId, "Staff Engineer",    isValueId: false, isFullText: true);
        await InsertAsync(conn, "department",   AlicePersonId, "Engineering",       isValueId: false, isFullText: true);
    }

    private static async Task InsertAsync(
        MySqlConnection conn,
        string valueType,
        Guid personId,
        string value,
        bool isValueId,
        bool isFullText = false)
    {
        const string sql = """
            INSERT IGNORE INTO persons
                (value_type, insight_source_type, insight_source_id, insight_tenant_id,
                 value_id, value_full_text, value,
                 person_id, author_person_id, reason, created_at)
            VALUES
                (@vt, 'bamboohr', @src, @tenant,
                 @vid, @vft, @vraw,
                 @person, @author, '', UTC_TIMESTAMP(6))
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@vt", valueType);
        cmd.Parameters.AddWithValue("@src", SourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant", TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@vid", isValueId ? value : DBNull.Value);
        cmd.Parameters.AddWithValue("@vft", isFullText ? value : DBNull.Value);
        cmd.Parameters.AddWithValue("@vraw", (!isValueId && !isFullText) ? value : DBNull.Value);
        cmd.Parameters.AddWithValue("@person", personId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@author", AuthorPersonId.ToByteArray(bigEndian: true));
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }
}
