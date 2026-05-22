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
    private static readonly Guid TenantId = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid SourceId = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    private static readonly Guid AlicePersonId = Guid.Parse("cccccccc-cccc-cccc-cccc-cccccccccccc");
    private static readonly Guid AuthorPersonId = Guid.Empty;

    private readonly MariaDbFixture _fixture;
    private TestApplicationFactory? _app;

    public PersonsEndpointTests(MariaDbFixture fixture) => _fixture = fixture;

    public async Task InitializeAsync()
    {
        await _fixture.ResetAsync().ConfigureAwait(false);
        _app = new TestApplicationFactory(_fixture.ConnectionString, TenantId);
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
        var doc = await response.Content.ReadFromJsonAsync<JsonElement>().ConfigureAwait(false);

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
        // No header sent and no default tenant configured → composite
        // resolver returns null → endpoint must respond 400.
        using var noTenantApp = new TestApplicationFactory(_fixture.ConnectionString, defaultTenantId: null);
        var client = noTenantApp.CreateClient();
        client.DefaultRequestHeaders.Remove(HeaderTenantContext.HeaderName);

        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
    }

    [Fact]
    public async Task Resolves_tenant_from_jwt_insight_tenant_id_claim()
    {
        // Proves the JwtBearer middleware wires the bearer payload into
        // `HttpContext.User` so `JwtTenantContext` can read the claim.
        // No X-Insight-Tenant-Id header, no config default — the only
        // path to a tenant is the JWT.
        await SeedAliceAsync().ConfigureAwait(false);
        using var jwtApp = new TestApplicationFactory(_fixture.ConnectionString, defaultTenantId: null);
        var client = jwtApp.CreateClient();
        client.DefaultRequestHeaders.Authorization =
            new System.Net.Http.Headers.AuthenticationHeaderValue("Bearer", BuildUnverifiedJwt(TenantId));

        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        if (!response.IsSuccessStatusCode)
        {
            var body = await response.Content.ReadAsStringAsync().ConfigureAwait(false);
            throw new InvalidOperationException($"Expected 200, got {(int)response.StatusCode}. Body: {body}");
        }
        var doc = await response.Content.ReadFromJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("email").GetString().Should().Be("alice@example.com");
    }

    private static string BuildUnverifiedJwt(Guid tenantId)
    {
        // Hand-crafted token: parse-only middleware ignores the signature
        // segment so we can omit a real signing key. `alg=HS256` keeps
        // the header shape conventional; the third segment is a
        // non-empty placeholder (`AAAA`) — `JsonWebToken` requires three
        // dot-separated segments and tolerates an opaque signature when
        // the SignatureValidator short-circuits.
        static string B64Url(string raw)
        {
            var bytes = System.Text.Encoding.UTF8.GetBytes(raw);
            return Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');
        }
        var header = B64Url("{\"alg\":\"HS256\",\"typ\":\"JWT\"}");
        var payload = B64Url($"{{\"insight_tenant_id\":\"{tenantId:D}\"}}");
        return $"{header}.{payload}.AAAA";
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
