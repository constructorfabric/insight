using FluentAssertions;
using Insight.Identity.Domain;
using Insight.Identity.Domain.Services;
using Insight.Identity.Infrastructure.MariaDb;
using MySqlConnector;
using Xunit;

namespace Insight.Identity.Tests.Integration;

[Collection(MariaDbCollection.Name)]
public sealed class OrgTreeLookupTests : IAsyncLifetime
{
    private static readonly Guid TenantId       = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid BambooSourceId = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    private static readonly Guid SlackSourceId  = Guid.Parse("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee");

    private static readonly Guid AlicePersonId  = Guid.Parse("11111111-1111-1111-1111-111111111111");
    private static readonly Guid BobPersonId    = Guid.Parse("22222222-2222-2222-2222-222222222222");
    private static readonly Guid CarolPersonId  = Guid.Parse("33333333-3333-3333-3333-333333333333");
    private static readonly Guid DavePersonId   = Guid.Parse("44444444-4444-4444-4444-444444444444");
    private static readonly Guid AuthorPersonId = Guid.Empty;

    private static readonly LookupOptions Options = LookupOptions.Default;

    private readonly MariaDbFixture _fixture;
    private PersonsRepository? _repo;
    private PersonLookupService? _personLookup;
    private ProfileLookupService? _profileLookup;

    public OrgTreeLookupTests(MariaDbFixture fixture) => _fixture = fixture;

    public async Task InitializeAsync()
    {
        await _fixture.ResetAsync().ConfigureAwait(false);
        _repo = new PersonsRepository(new MariaDbConnectionFactory(
            new Microsoft.Extensions.Options.OptionsWrapper<MariaDbOptions>(
                new MariaDbOptions { ConnectionString = _fixture.ConnectionString })));
        _personLookup = new PersonLookupService(_repo);
        _profileLookup = new ProfileLookupService(_repo, _personLookup);
    }

    public Task DisposeAsync() => Task.CompletedTask;

    // ── /v1/persons/{email} ──────────────────────────────────────────

    [Fact]
    public async Task GetByEmail_returns_person_with_no_parent_when_org_chart_empty()
    {
        await SeedPersonAsync(BobPersonId, "bamboohr", BambooSourceId,
            email: "bob@example.com",
            displayName: "Bob Jones",
            jobTitle: "VP Engineering").ConfigureAwait(false);

        var bob = await _personLookup!.GetByEmailAsync(TenantId, "bob@example.com", Options, CancellationToken.None);

        bob.Should().NotBeNull();
        bob!.Email.Should().Be("bob@example.com");
        bob.SupervisorEmail.Should().BeNull();
        bob.SupervisorName.Should().BeNull();
        bob.ParentEmail.Should().BeNull();
        bob.ParentPersonId.Should().BeNull();
        bob.Subordinates.Should().BeEmpty();
    }

    [Fact]
    public async Task GetByEmail_hydrates_supervisor_from_parent_observations_via_org_chart()
    {
        // Alice reports to Bob (BambooHR). Bob's own record holds the
        // supervisor's email and display name; the assembler reads them
        // through the parent edge resolver, not from Alice's
        // value_type='parent_*' observations.
        await SeedPersonAsync(BobPersonId, "bamboohr", BambooSourceId,
            email: "bob@example.com",
            displayName: "Jones, Bob",
            jobTitle: "Engineering Manager").ConfigureAwait(false);
        await SeedSourceIdAsync(BobPersonId, "bamboohr", BambooSourceId, "BOB-7").ConfigureAwait(false);
        await SeedPersonAsync(AlicePersonId, "bamboohr", BambooSourceId,
            email: "alice@example.com",
            displayName: "Alice Smith",
            jobTitle: "Staff Engineer").ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);

        var alice = await _personLookup!.GetByEmailAsync(TenantId, "alice@example.com", Options, CancellationToken.None);

        alice.Should().NotBeNull();
        alice!.SupervisorEmail.Should().Be("bob@example.com");
        alice.SupervisorName.Should().Be("Jones, Bob");
        // Legacy fields mirror the same edge (BambooHR-scoped).
        alice.ParentEmail.Should().Be("bob@example.com");
        alice.ParentId.Should().Be("BOB-7");
        alice.ParentPersonId.Should().Be(BobPersonId);
    }

    [Fact]
    public async Task GetByEmail_ignores_non_bamboohr_org_chart_edges()
    {
        // Alice has a parent in BambooHR (Bob) AND a different parent in
        // Slack (Carol). Only BambooHR drives the response; the Slack
        // edge must not surface.
        await SeedPersonAsync(BobPersonId, "bamboohr", BambooSourceId,
            email: "bob@example.com",
            displayName: "Jones, Bob",
            jobTitle: "Engineering Manager").ConfigureAwait(false);
        await SeedPersonAsync(CarolPersonId, "slack", SlackSourceId,
            email: "carol@example.com",
            displayName: "Carol Lee",
            jobTitle: "Slack Admin").ConfigureAwait(false);
        await SeedPersonAsync(AlicePersonId, "bamboohr", BambooSourceId,
            email: "alice@example.com",
            displayName: "Alice Smith",
            jobTitle: "Staff Engineer").ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);
        await InsertEdgeAsync("slack", SlackSourceId, child: AlicePersonId, parent: CarolPersonId).ConfigureAwait(false);

        var alice = await _personLookup!.GetByEmailAsync(TenantId, "alice@example.com", Options, CancellationToken.None);

        alice!.SupervisorEmail.Should().Be("bob@example.com");
        alice.ParentPersonId.Should().Be(BobPersonId);
        alice.SupervisorName.Should().NotContain("Carol");
    }

    [Fact]
    public async Task GetByEmail_recursively_walks_bamboohr_subordinates()
    {
        // Tree: Carol → Bob → (Alice, Dave). All in BambooHR.
        await SeedPersonAsync(CarolPersonId, "bamboohr", BambooSourceId,
            email: "carol@example.com",
            displayName: "Carol Lee",
            jobTitle: "VP Engineering").ConfigureAwait(false);
        await SeedPersonAsync(BobPersonId, "bamboohr", BambooSourceId,
            email: "bob@example.com",
            displayName: "Jones, Bob",
            jobTitle: "Engineering Manager").ConfigureAwait(false);
        await SeedPersonAsync(AlicePersonId, "bamboohr", BambooSourceId,
            email: "alice@example.com",
            displayName: "Alice Smith",
            jobTitle: "Staff Engineer").ConfigureAwait(false);
        await SeedPersonAsync(DavePersonId, "bamboohr", BambooSourceId,
            email: "dave@example.com",
            displayName: "Dave Ng",
            jobTitle: "Senior Engineer").ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: BobPersonId, parent: CarolPersonId).ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: DavePersonId, parent: BobPersonId).ConfigureAwait(false);

        var carol = await _personLookup!.GetByEmailAsync(TenantId, "carol@example.com", Options, CancellationToken.None);

        carol.Should().NotBeNull();
        carol!.Subordinates.Should().HaveCount(1);
        var bob = carol.Subordinates[0];
        bob.Email.Should().Be("bob@example.com");
        bob.Subordinates.Should().HaveCount(2);
        var expectedEmails = new[] { "alice@example.com", "dave@example.com" };
        bob.Subordinates.Select(s => s.Email).Should().BeEquivalentTo(expectedEmails);
        // Leaves have empty subordinates lists.
        bob.Subordinates.Should().AllSatisfy(s => s.Subordinates.Should().BeEmpty());
    }

    [Fact]
    public async Task GetByEmail_excludes_non_bamboohr_children_from_subordinates()
    {
        // Bob has Alice as a BambooHR direct report and Carol as a Slack
        // "report" (channel admin). Subordinates should contain only Alice.
        await SeedPersonAsync(BobPersonId, "bamboohr", BambooSourceId,
            email: "bob@example.com",
            displayName: "Jones, Bob",
            jobTitle: "Engineering Manager").ConfigureAwait(false);
        await SeedPersonAsync(AlicePersonId, "bamboohr", BambooSourceId,
            email: "alice@example.com",
            displayName: "Alice Smith",
            jobTitle: "Staff Engineer").ConfigureAwait(false);
        await SeedPersonAsync(CarolPersonId, "slack", SlackSourceId,
            email: "carol@example.com",
            displayName: "Carol Lee",
            jobTitle: "Channel Admin").ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);
        await InsertEdgeAsync("slack", SlackSourceId, child: CarolPersonId, parent: BobPersonId).ConfigureAwait(false);

        var bob = await _personLookup!.GetByEmailAsync(TenantId, "bob@example.com", Options, CancellationToken.None);

        bob!.Subordinates.Should().HaveCount(1);
        bob.Subordinates[0].Email.Should().Be("alice@example.com");
    }

    [Fact]
    public async Task GetByEmail_breaks_on_cycle()
    {
        // Pathological cycle: Alice → Bob → Alice (same source instance).
        // The seeder's two-hop check would WARN on this; the read path
        // must not loop forever — visited-set protection bounds the walk.
        await SeedPersonAsync(AlicePersonId, "bamboohr", BambooSourceId,
            email: "alice@example.com",
            displayName: "Alice Smith",
            jobTitle: "Staff Engineer").ConfigureAwait(false);
        await SeedPersonAsync(BobPersonId, "bamboohr", BambooSourceId,
            email: "bob@example.com",
            displayName: "Jones, Bob",
            jobTitle: "Engineering Manager").ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: BobPersonId, parent: AlicePersonId).ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);

        var alice = await _personLookup!.GetByEmailAsync(TenantId, "alice@example.com", Options, CancellationToken.None);

        // Alice reports up to Bob; recursive subordinates would walk
        // back to Alice via the cycle but the visited set stops there.
        // We only assert the call returns within bounded time/payload.
        alice.Should().NotBeNull();
        alice!.Subordinates.Should().HaveCount(1);
        alice.Subordinates[0].Email.Should().Be("bob@example.com");
        // Bob's recursive subs hit Alice → cycle break → empty.
        alice.Subordinates[0].Subordinates.Should().BeEmpty();
    }

    // ── /v1/profiles ─────────────────────────────────────────────────

    [Fact]
    public async Task ResolveAsync_returns_same_tree_shape_as_person_lookup()
    {
        await SeedPersonAsync(BobPersonId, "bamboohr", BambooSourceId,
            email: "bob@example.com",
            displayName: "Jones, Bob",
            jobTitle: "Engineering Manager").ConfigureAwait(false);
        await SeedSourceIdAsync(BobPersonId, "bamboohr", BambooSourceId, "BOB-7").ConfigureAwait(false);
        await SeedPersonAsync(AlicePersonId, "bamboohr", BambooSourceId,
            email: "alice@example.com",
            displayName: "Alice Smith",
            jobTitle: "Staff Engineer").ConfigureAwait(false);
        await SeedSourceIdAsync(AlicePersonId, "bamboohr", BambooSourceId, "ALICE-1").ConfigureAwait(false);
        await InsertEdgeAsync("bamboohr", BambooSourceId, child: AlicePersonId, parent: BobPersonId).ConfigureAwait(false);

        var query = new ResolveProfileQuery(ResolveProfileKind.Email, "alice@example.com", null, null);
        var result = await _profileLookup!.ResolveAsync(TenantId, query, Options, CancellationToken.None);

        result.Should().BeOfType<ProfileLookupResult.Found>();
        var profile = ((ProfileLookupResult.Found)result).Profile;

        profile.PersonId.Should().Be(AlicePersonId);
        profile.SupervisorEmail.Should().Be("bob@example.com");
        profile.SupervisorName.Should().Be("Jones, Bob");
        profile.ParentEmail.Should().Be("bob@example.com");
        profile.ParentId.Should().Be("BOB-7");
        profile.ParentPersonId.Should().Be(BobPersonId);
        profile.Ids.Should().HaveCount(1);
        profile.Ids[0].Value.Should().Be("ALICE-1");
    }

    // ── Seed helpers ─────────────────────────────────────────────────

    private async Task SeedPersonAsync(
        Guid personId,
        string sourceType,
        Guid sourceId,
        string email,
        string displayName,
        string jobTitle)
    {
        // One row per value_type — enough to drive the assembler.
        await InsertObservationAsync(personId, sourceType, sourceId, "email", email).ConfigureAwait(false);
        await InsertObservationAsync(personId, sourceType, sourceId, "display_name", displayName).ConfigureAwait(false);
        await InsertObservationAsync(personId, sourceType, sourceId, "job_title", jobTitle).ConfigureAwait(false);
    }

    private async Task SeedSourceIdAsync(Guid personId, string sourceType, Guid sourceId, string nativeId)
    {
        await InsertObservationAsync(personId, sourceType, sourceId, "id", nativeId).ConfigureAwait(false);
    }

    private async Task InsertObservationAsync(
        Guid personId,
        string sourceType,
        Guid sourceId,
        string valueType,
        string value)
    {
        // ADR-0007: route by value_type. Identifier-like fields land in
        // value_id (indexed, case-insensitive collation per ADR-0011);
        // display_name lands in value_full_text; free-form fields land
        // in the value TEXT column.
        var col = valueType switch
        {
            "email" or "id" or "username" => "value_id",
            "display_name" => "value_full_text",
            _ => "value",
        };
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        var sql = $"""
            INSERT INTO persons
                (insight_tenant_id, insight_source_type, insight_source_id,
                 person_id, author_person_id, value_type, {col}, reason)
            VALUES (@t, @st, @sid, @pid, @a, @vt, @val, '')
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@t",   TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@st",  sourceType);
        cmd.Parameters.AddWithValue("@sid", sourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@pid", personId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@a",   AuthorPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@vt",  valueType);
        cmd.Parameters.AddWithValue("@val", value);
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }

    private async Task InsertEdgeAsync(
        string sourceType,
        Guid sourceId,
        Guid child,
        Guid parent,
        DateTime? validFrom = null)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            INSERT INTO org_chart
                (insight_tenant_id, insight_source_type, insight_source_id,
                 child_person_id, parent_person_id, author_person_id, reason,
                 valid_from, valid_to)
            VALUES (@t, @st, @sid, @c, @p, @a, '', @vf, NULL)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@t",   TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@st",  sourceType);
        cmd.Parameters.AddWithValue("@sid", sourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@c",   child.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@p",   parent.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@a",   AuthorPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@vf",  validFrom ?? new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc));
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }
}
