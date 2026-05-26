using FluentAssertions;
using Insight.Identity.Domain;
using Insight.Identity.Domain.Services;
using Xunit;

namespace Insight.Identity.Tests.Unit;

public sealed class ProfileLookupServiceTests
{
    private static readonly Guid TenantId = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid PersonId = Guid.Parse("cccccccc-cccc-cccc-cccc-cccccccccccc");
    private static readonly Guid SourceId = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");

    private static readonly LookupOptions Options = LookupOptions.Default;

    [Fact]
    public async Task Returns_NotFound_when_resolver_returns_empty_list()
    {
        var reader = new StubReader { ResolveEmail = Array.Empty<Guid>() };
        var svc = new ProfileLookupService(reader, new PersonLookupService(reader));

        var result = await svc.ResolveAsync(
            TenantId,
            new ResolveProfileQuery(ResolveProfileKind.Email, "ghost@nowhere.test", null, null),
            Options,
            CancellationToken.None);

        result.Should().BeOfType<ProfileLookupResult.NotFound>();
    }

    [Fact]
    public async Task Returns_Ambiguous_when_resolver_returns_multiple_person_ids()
    {
        var ids = new[] { PersonId, Guid.Parse("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee") };
        var reader = new StubReader { ResolveEmail = ids };
        var svc = new ProfileLookupService(reader, new PersonLookupService(reader));

        var result = await svc.ResolveAsync(
            TenantId,
            new ResolveProfileQuery(ResolveProfileKind.Email, "shared@example.test", null, null),
            Options,
            CancellationToken.None);

        result.Should().BeOfType<ProfileLookupResult.Ambiguous>()
            .Which.PersonIds.Should().BeEquivalentTo(ids);
    }

    [Fact]
    public async Task Returns_NotFound_when_resolver_succeeds_but_hydration_is_empty()
    {
        var reader = new StubReader
        {
            ResolveEmail = new[] { PersonId },
            LatestObservations = Array.Empty<PersonObservation>(),
            CurrentSourceIds = Array.Empty<PersonSourceId>(),
        };
        var svc = new ProfileLookupService(reader, new PersonLookupService(reader));

        var result = await svc.ResolveAsync(
            TenantId,
            new ResolveProfileQuery(ResolveProfileKind.Email, "ghost@example.test", null, null),
            Options,
            CancellationToken.None);

        result.Should().BeOfType<ProfileLookupResult.NotFound>();
    }

    [Fact]
    public async Task Routes_to_source_id_resolver_for_id_lookups()
    {
        var reader = new StubReader { ResolveSourceId = new[] { PersonId } };
        var svc = new ProfileLookupService(reader, new PersonLookupService(reader));

        var query = new ResolveProfileQuery(
            ResolveProfileKind.SourceId,
            "alice-bamboo-001",
            SourceType: "bamboohr",
            SourceId: SourceId);

        var result = await svc.ResolveAsync(TenantId, query, Options, CancellationToken.None);

        result.Should().BeOfType<ProfileLookupResult.NotFound>();
        reader.SourceIdCalls.Should().Be(1);
        reader.EmailCalls.Should().Be(0);
    }

    private sealed class StubReader : IPersonsReader
    {
        public IReadOnlyList<Guid> ResolveEmail { get; init; } = Array.Empty<Guid>();
        public IReadOnlyList<Guid> ResolveSourceId { get; init; } = Array.Empty<Guid>();
        public IReadOnlyList<PersonObservation> LatestObservations { get; init; } = Array.Empty<PersonObservation>();
        public IReadOnlyList<PersonSourceId> CurrentSourceIds { get; init; } = Array.Empty<PersonSourceId>();

        public int EmailCalls { get; private set; }
        public int SourceIdCalls { get; private set; }

        public Task<IReadOnlyList<Guid>> ResolvePersonIdsByEmailAsync(Guid tenantId, string email, CancellationToken cancellationToken)
        {
            EmailCalls++;
            return Task.FromResult(ResolveEmail);
        }

        public Task<IReadOnlyList<Guid>> ResolvePersonIdsBySourceIdAsync(Guid tenantId, string sourceType, Guid sourceId, string value, CancellationToken cancellationToken)
        {
            SourceIdCalls++;
            return Task.FromResult(ResolveSourceId);
        }

        public Task<IReadOnlyList<PersonObservation>> GetLatestObservationsAsync(Guid tenantId, Guid personId, CancellationToken cancellationToken)
            => Task.FromResult(LatestObservations);

        public Task<IReadOnlyList<PersonSourceId>> GetCurrentSourceIdsAsync(Guid tenantId, Guid personId, CancellationToken cancellationToken)
            => Task.FromResult(CurrentSourceIds);

        public Task<Guid?> ResolvePersonIdByEmailAsync(Guid tenantId, string email, CancellationToken cancellationToken)
            => Task.FromResult<Guid?>(null);

        public Task<IReadOnlyList<OrgChartEdge>> GetCurrentParentsAsync(Guid tenantId, Guid childPersonId, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<OrgChartEdge>>(Array.Empty<OrgChartEdge>());

        public Task<IReadOnlyList<OrgChartEdge>> GetCurrentChildrenAsync(Guid tenantId, Guid parentPersonId, CancellationToken cancellationToken)
            => Task.FromResult<IReadOnlyList<OrgChartEdge>>(Array.Empty<OrgChartEdge>());

        public Task<Guid?> ResolvePersonIdByAccountIdAsync(Guid tenantId, string accountId, CancellationToken cancellationToken)
            => Task.FromResult<Guid?>(null);
    }
}
