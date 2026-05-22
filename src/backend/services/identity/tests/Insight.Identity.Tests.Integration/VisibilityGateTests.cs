using System.Net;
using System.Net.Http.Json;
using FluentAssertions;
using Insight.Identity.Api.Contracts;
using MySqlConnector;
using Xunit;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// End-to-end tests for the visibility gate on <c>/v1/persons/{email}</c>
/// and <c>POST /v1/profiles</c>. Each test builds its own seed (caller
/// and target persons, optional grants, org_chart edges) and verifies
/// the can-A-see-B predicate against the recursive CTE.
/// </summary>
[Collection(MariaDbCollection.Name)]
public sealed class VisibilityGateTests : IAsyncLifetime
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

    public VisibilityGateTests(MariaDbFixture fixture) => _fixture = fixture;

    public async Task InitializeAsync()
    {
        await _fixture.ResetAsync().ConfigureAwait(false);
        // Seed the tree Carol → Bob → (Alice, Dave) and the outsider —
        // no visibility grants by default; each test wires what it needs.
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

    // ── Self ────────────────────────────────────────────────────────

    [Fact]
    public async Task Self_lookup_succeeds_without_any_grant()
    {
        // Alice queries herself, no visibility row anywhere — the
        // self short-circuit in VisibilityService returns true.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: AlicePersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.OK);
    }

    // ── Whole-tenant grant ──────────────────────────────────────────

    [Fact]
    public async Task Whole_tenant_grant_lets_caller_see_anyone()
    {
        // Outsider has a whole-tenant grant (viewed_person_id IS NULL).
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        await _fixture.SeedWholeTenantVisibilityAsync(TenantId, OutsiderPersonId).ConfigureAwait(false);
        var client = app.CreateClient();

        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.OK);
    }

    // ── Subtree grant ───────────────────────────────────────────────

    [Fact]
    public async Task Subtree_grant_on_parent_lets_caller_see_descendants()
    {
        // Outsider gets an explicit grant on Bob → can see Bob's
        // subtree (Bob, Alice, Dave) but NOT Carol (Bob's parent).
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        await InsertVisibilityAsync(viewerPersonId: OutsiderPersonId, viewedPersonId: BobPersonId).ConfigureAwait(false);
        var client = app.CreateClient();

        (await client.GetAsync(new Uri("/v1/persons/bob@example.com",   UriKind.Relative)).ConfigureAwait(false))
            .StatusCode.Should().Be(HttpStatusCode.OK);
        (await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative)).ConfigureAwait(false))
            .StatusCode.Should().Be(HttpStatusCode.OK);
        (await client.GetAsync(new Uri("/v1/persons/dave@example.com",  UriKind.Relative)).ConfigureAwait(false))
            .StatusCode.Should().Be(HttpStatusCode.OK);
        (await client.GetAsync(new Uri("/v1/persons/carol@example.com", UriKind.Relative)).ConfigureAwait(false))
            .StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    // ── Org_chart descent of caller ─────────────────────────────────

    [Fact]
    public async Task Caller_sees_own_descendants_via_org_chart_without_explicit_grant()
    {
        // Bob has no visibility row. The CTE seeds him as the viewer,
        // then walks org_chart down to Alice and Dave (his reports).
        // Carol (Bob's parent) is upward — not in the visible set.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: BobPersonId);
        var client = app.CreateClient();

        (await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative)).ConfigureAwait(false))
            .StatusCode.Should().Be(HttpStatusCode.OK);
        (await client.GetAsync(new Uri("/v1/persons/dave@example.com",  UriKind.Relative)).ConfigureAwait(false))
            .StatusCode.Should().Be(HttpStatusCode.OK);
        (await client.GetAsync(new Uri("/v1/persons/carol@example.com", UriKind.Relative)).ConfigureAwait(false))
            .StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    // ── Deny → 404 (no existence leak) ──────────────────────────────

    [Fact]
    public async Task Outsider_with_no_grant_gets_404_on_existing_target()
    {
        // Outsider has zero rows in visibility and isn't in org_chart.
        // Alice exists in the tenant but is invisible to outsider —
        // response shape is identical to "no such person" (404).
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        var client = app.CreateClient();

        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    // ── Soft-delete excludes the grant ──────────────────────────────

    [Fact]
    public async Task Revoked_grant_is_ignored()
    {
        // Outsider once had a grant on Bob but it was revoked
        // (valid_to set in the past). The CTE filters by valid_to IS
        // NULL — revoked rows must not contribute to the visible set.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        await InsertVisibilityAsync(
            viewerPersonId: OutsiderPersonId,
            viewedPersonId: BobPersonId,
            validTo: new DateTime(2024, 1, 1, 0, 0, 0, DateTimeKind.Utc)).ConfigureAwait(false);
        var client = app.CreateClient();

        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    // ── Cross-tenant ────────────────────────────────────────────────

    [Fact]
    public async Task Grant_in_other_tenant_does_not_apply()
    {
        // Outsider has a whole-tenant grant in OtherTenantId but the
        // request comes for TenantId — the CTE is tenant-scoped, so
        // the foreign grant is invisible.
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        await _fixture.SeedWholeTenantVisibilityAsync(OtherTenantId, OutsiderPersonId).ConfigureAwait(false);
        var client = app.CreateClient();

        var response = await client.GetAsync(new Uri("/v1/persons/alice@example.com", UriKind.Relative))
            .ConfigureAwait(false);
        response.StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    // ── POST /v1/profiles parity ────────────────────────────────────

    [Fact]
    public async Task Profile_lookup_applies_same_gate()
    {
        using var app = new TestApplicationFactory(
            _fixture.ConnectionString, TenantId, defaultCallerPersonId: OutsiderPersonId);
        var client = app.CreateClient();
        var body = new ResolveProfileCommandModel("email", "alice@example.com", null, null);

        var response = await client.PostAsJsonAsync(new Uri("/v1/profiles", UriKind.Relative), body)
            .ConfigureAwait(false);
        // Outsider has no visibility — must look exactly like
        // "no current observation matches" so existence doesn't leak.
        response.StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    // ── Seed helpers ────────────────────────────────────────────────

    private async Task SeedPersonAsync(Guid personId, string email, string displayName)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        await InsertObservationAsync(conn, personId, "email",        email);
        await InsertObservationAsync(conn, personId, "display_name", displayName);
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
        cmd.Parameters.AddWithValue("@vt", valueType);
        cmd.Parameters.AddWithValue("@src", BambooSourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant", TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@val", value);
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

    private async Task InsertVisibilityAsync(Guid viewerPersonId, Guid? viewedPersonId, DateTime? validTo = null)
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
        cmd.Parameters.AddWithValue("@viewer",   viewerPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@viewed",   viewedPersonId is { } v ? v.ToByteArray(bigEndian: true) : (object)DBNull.Value);
        cmd.Parameters.AddWithValue("@valid_to", validTo is { } t ? t : (object)DBNull.Value);
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }
}
