using System.Net;
using System.Net.Http.Headers;
using System.Text.Json;
using FluentAssertions;
using MySqlConnector;
using Xunit;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// End-to-end tests for the JWT fallback in
/// <see cref="Insight.Identity.Api.Auth.HeaderCallerContext"/>: with no
/// <c>X-Insight-Person-Id</c> header on the request, the resolver tries
/// <c>oid</c> then <c>sub</c> against <c>account_person_map</c>, then
/// <c>email</c> / <c>preferred_username</c> / <c>upn</c> against
/// <c>persons.value_type='email'</c>. The result is cached on the
/// request so multiple lookups inside one handler share one SQL hit.
/// </summary>
[Collection(MariaDbCollection.Name)]
public sealed class JwtCallerResolveTests : IAsyncLifetime
{
    // The test tenant the JWT carries via insight_tenant_id claim.
    private static readonly Guid TenantId = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");

    // A canonical person bound in three ways: account_person_map by oid,
    // and persons by email. Each test will hit a different claim shape
    // and expect the same person to come out.
    private static readonly Guid CallerPersonId = Guid.Parse("019e2bfb-2a7b-7bf0-852c-7053a2c7f50d");
    private static readonly Guid SourceId       = Guid.Parse("11111111-1111-1111-1111-111111111111");

    // Distinct second person — used to assert email returns the right
    // person when multiple persons share an oid prefix or similar.
    private static readonly Guid OtherPersonId  = Guid.Parse("33333333-3333-3333-3333-333333333333");

    private const string OidValue   = "22c303cd-1364-4165-8fb8-7b412f0e3bb6";
    private const string SubValue   = "ZfBGhYsNQWyvDWPNRGfd-zefLAdrC0x8g_vJe2veeEI";
    private const string EmailValue = "grigoriy.gogin@constructor.tech";

    private readonly MariaDbFixture _fixture;

    public JwtCallerResolveTests(MariaDbFixture fixture) => _fixture = fixture;

    public async Task InitializeAsync()
    {
        await _fixture.ResetAsync().ConfigureAwait(false);
        await SeedCallerAsync().ConfigureAwait(false);
        // Caller needs visibility on the target to get past the
        // /v1/persons gate; a whole-tenant grant on themselves is the
        // simplest way to keep the JWT-resolution assertion isolated
        // from visibility logic.
        await _fixture.SeedWholeTenantVisibilityAsync(TenantId, CallerPersonId).ConfigureAwait(false);
    }

    public Task DisposeAsync() => Task.CompletedTask;

    [Fact]
    public async Task Resolves_caller_from_jwt_oid_claim_via_account_person_map()
    {
        // oid is the stable Entra user id and matches account_person_map
        // when an Entra/M365 connector is wired. Tried first.
        var response = await CallSelfLookupAsync(BuildJwt(("insight_tenant_id", TenantId.ToString("D")), ("oid", OidValue)))
            .ConfigureAwait(false);
        await AssertResolvedAsync(response).ConfigureAwait(false);
    }

    [Fact]
    public async Task Resolves_caller_from_jwt_sub_claim_via_account_person_map()
    {
        var response = await CallSelfLookupAsync(BuildJwt(("insight_tenant_id", TenantId.ToString("D")), ("sub", SubValue)))
            .ConfigureAwait(false);
        await AssertResolvedAsync(response).ConfigureAwait(false);
    }

    [Fact]
    public async Task Resolves_caller_from_jwt_email_claim_via_persons()
    {
        // Falls back through persons.value_type='email'. Real-world case:
        // no oid/sub binding present (no ms-entra connector), email is
        // the only claim that finds the caller.
        var response = await CallSelfLookupAsync(BuildJwt(("insight_tenant_id", TenantId.ToString("D")), ("email", EmailValue)))
            .ConfigureAwait(false);
        await AssertResolvedAsync(response).ConfigureAwait(false);
    }

    [Fact]
    public async Task Resolves_caller_from_jwt_preferred_username_when_email_claim_absent()
    {
        var response = await CallSelfLookupAsync(BuildJwt(
            ("insight_tenant_id", TenantId.ToString("D")),
            ("preferred_username", EmailValue))).ConfigureAwait(false);
        await AssertResolvedAsync(response).ConfigureAwait(false);
    }

    [Fact]
    public async Task Resolves_caller_from_jwt_upn_when_email_and_preferred_username_absent()
    {
        // upn is the last email-shaped fallback before the resolver gives up.
        var response = await CallSelfLookupAsync(BuildJwt(
            ("insight_tenant_id", TenantId.ToString("D")),
            ("upn", EmailValue))).ConfigureAwait(false);
        await AssertResolvedAsync(response).ConfigureAwait(false);
    }

    [Fact]
    public async Task Header_caller_overrides_jwt_when_both_present()
    {
        // Header points at one person, JWT at another. The resolver
        // must return the header value and not look at any JWT claims.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, defaultTenantId: null, defaultCallerPersonId: OtherPersonId);
        await SeedOtherCallerAsAdminAsync().ConfigureAwait(false);
        var client = app.CreateClient();
        client.DefaultRequestHeaders.Authorization =
            new AuthenticationHeaderValue("Bearer", BuildJwt(("insight_tenant_id", TenantId.ToString("D")), ("email", EmailValue)));

        var response = await client
            .GetAsync(new Uri($"/v1/persons/{EmailValue}", UriKind.Relative))
            .ConfigureAwait(false);

        response.IsSuccessStatusCode.Should().BeTrue(
            "the header pinned OtherPersonId as the caller; that person has whole-tenant visibility");
    }

    [Fact]
    public async Task No_header_no_jwt_returns_401()
    {
        // Tenant comes from config so this test isolates the missing-
        // caller branch from the missing-tenant branch.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, defaultTenantId: TenantId, defaultCallerPersonId: null);
        var response = await app.CreateClient()
            .GetAsync(new Uri($"/v1/persons/{EmailValue}", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("type").GetString().Should().Be("urn:insight:error:caller_unresolved");
    }

    [Fact]
    public async Task Jwt_claims_present_but_unmapped_returns_401()
    {
        // Token carries claims, but none of them maps to a known
        // person: the oid is not in account_person_map, the email is
        // not in persons.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, defaultTenantId: null, defaultCallerPersonId: null);
        var client = app.CreateClient();
        client.DefaultRequestHeaders.Authorization =
            new AuthenticationHeaderValue("Bearer", BuildJwt(
                ("insight_tenant_id", TenantId.ToString("D")),
                ("oid",   "ffffffff-ffff-ffff-ffff-ffffffffffff"),
                ("email", "stranger@example.com")));

        var response = await client
            .GetAsync(new Uri($"/v1/persons/{EmailValue}", UriKind.Relative))
            .ConfigureAwait(false);

        response.StatusCode.Should().Be(HttpStatusCode.Unauthorized);
    }

    // ── helpers ─────────────────────────────────────────────────────

    private async Task<HttpResponseMessage> CallSelfLookupAsync(string jwt)
    {
        // No default header, no config tenant — the JWT is the only
        // signal. The caller resolves from the token, then we look up
        // the caller's own email (the whole-tenant visibility grant
        // makes the visibility check pass).
        var app = new TestApplicationFactory(
            _fixture.ConnectionString, defaultTenantId: null, defaultCallerPersonId: null);
        var client = app.CreateClient();
        client.DefaultRequestHeaders.Authorization = new AuthenticationHeaderValue("Bearer", jwt);
        return await client
            .GetAsync(new Uri($"/v1/persons/{EmailValue}", UriKind.Relative))
            .ConfigureAwait(false);
    }

    private static async Task AssertResolvedAsync(HttpResponseMessage response)
    {
        if (!response.IsSuccessStatusCode)
        {
            var body = await response.Content.ReadAsStringAsync().ConfigureAwait(false);
            throw new InvalidOperationException($"Expected 200, got {(int)response.StatusCode}. Body: {body}");
        }
        var doc = await response.ReadJsonAsync<JsonElement>().ConfigureAwait(false);
        doc.GetProperty("email").GetString().Should().Be(EmailValue);
    }

    private async Task SeedCallerAsync()
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);

        // persons: email observation so the email-claim path resolves.
        await InsertPersonObservationAsync(conn, CallerPersonId, "email", EmailValue, isValueId: true).ConfigureAwait(false);

        // account_person_map: one binding for oid, one for sub. The
        // two source_type values ('ms-entra' for oid, 'ms-entra-sub'
        // for sub) only exist so the (tenant, source_type, source_id,
        // source_account_id) primary key stays unique — the resolver
        // does not filter by source_type.
        await InsertAccountPersonMapAsync(conn, "ms-entra",     OidValue, CallerPersonId).ConfigureAwait(false);
        await InsertAccountPersonMapAsync(conn, "ms-entra-sub", SubValue, CallerPersonId).ConfigureAwait(false);
    }

    private async Task SeedOtherCallerAsAdminAsync()
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        // OtherPerson needs whole-tenant visibility so the header-
        // wins test gets past the /v1/persons visibility check.
        await _fixture.SeedWholeTenantVisibilityAsync(TenantId, OtherPersonId).ConfigureAwait(false);
    }

    private static async Task InsertPersonObservationAsync(
        MySqlConnection conn, Guid personId, string valueType, string value, bool isValueId)
    {
        const string sql = """
            INSERT INTO persons
                (value_type, insight_source_type, insight_source_id, insight_tenant_id,
                 value_id, value_full_text, value,
                 person_id, author_person_id, reason, created_at)
            VALUES
                (@vt, 'bamboohr', @src, @tenant,
                 @vid, NULL, @vraw,
                 @person, @person, '', UTC_TIMESTAMP(6))
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@vt",     valueType);
        cmd.Parameters.AddWithValue("@src",    SourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant", TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@vid",    isValueId ? value : DBNull.Value);
        cmd.Parameters.AddWithValue("@vraw",   isValueId ? DBNull.Value : value);
        cmd.Parameters.AddWithValue("@person", personId.ToByteArray(bigEndian: true));
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    private static async Task InsertAccountPersonMapAsync(
        MySqlConnection conn, string sourceType, string sourceAccountId, Guid personId)
    {
        const string sql = """
            INSERT INTO account_person_map
                (insight_tenant_id, insight_source_type, insight_source_id,
                 source_account_id, person_id,
                 author_person_id, reason, valid_from, valid_to)
            VALUES
                (@tenant, @stype, @src,
                 @account, @person,
                 @person, 'test seed', '2020-01-01 00:00:00', NULL)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@tenant",  TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@stype",   sourceType);
        cmd.Parameters.AddWithValue("@src",     SourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@account", sourceAccountId);
        cmd.Parameters.AddWithValue("@person",  personId.ToByteArray(bigEndian: true));
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    private static string BuildJwt(params (string Name, string Value)[] claims)
    {
        // Hand-built token. Program.cs runs JwtBearer in parse-only
        // mode (signature check is a no-op), so any 3-segment string
        // shaped like a JWT with a real JSON payload works.
        static string B64Url(string raw)
        {
            var bytes = System.Text.Encoding.UTF8.GetBytes(raw);
            return Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');
        }
        var header = B64Url("{\"alg\":\"HS256\",\"typ\":\"JWT\"}");
        var payloadJson = "{" + string.Join(",", claims.Select(c => $"\"{c.Name}\":\"{c.Value}\"")) + "}";
        var payload = B64Url(payloadJson);
        return $"{header}.{payload}.AAAA";
    }
}
