using FluentAssertions;
using Insight.Identity.Domain;
using Insight.Identity.Domain.Services;
using Xunit;

namespace Insight.Identity.Tests.Unit;

public sealed class ProfileAssemblerTests
{
    private static readonly Guid PersonId = Guid.Parse("11111111-1111-1111-1111-111111111111");
    private static readonly Guid TenantId = Guid.Parse("99999999-9999-9999-9999-999999999999");
    private static readonly Guid SourceId = Guid.Parse("22222222-2222-2222-2222-222222222222");

    private static PersonObservation Obs(string valueType, string value, string source = "bamboohr", DateTime? createdAt = null) =>
        new(PersonId, source, SourceId, valueType, value, createdAt ?? new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc));

    [Fact]
    public void Assembles_full_profile_with_ids_list()
    {
        var obs = new[]
        {
            Obs(ValueTypes.Email, "alice@example.com"),
            Obs(ValueTypes.DisplayName, "Alice Smith"),
            Obs(ValueTypes.FirstName, "Alice"),
            Obs(ValueTypes.LastName, "Smith"),
            Obs(ValueTypes.Department, "Eng"),
            Obs(ValueTypes.JobTitle, "SWE"),
        };
        var ids = new[]
        {
            new PersonSourceId("bamboohr", SourceId, "12345"),
            new PersonSourceId("slack", Guid.NewGuid(), "U03ABC"),
        };

        var profile = ProfileAssembler.Assemble(PersonId, TenantId, obs, ids);

        profile.PersonId.Should().Be(PersonId);
        profile.InsightTenantId.Should().Be(TenantId);
        profile.Email.Should().Be("alice@example.com");
        profile.DisplayName.Should().Be("Alice Smith");
        profile.FirstName.Should().Be("Alice");
        profile.LastName.Should().Be("Smith");
        profile.Department.Should().Be("Eng");
        profile.JobTitle.Should().Be("SWE");
        profile.Ids.Should().BeEquivalentTo(ids);
    }

    [Fact]
    public void Nulls_missing_optional_fields_instead_of_empty_strings()
    {
        var obs = new[] { Obs(ValueTypes.Email, "alice@example.com") };

        var profile = ProfileAssembler.Assemble(PersonId, TenantId, obs, Array.Empty<PersonSourceId>());

        profile.Department.Should().BeNull();
        profile.Division.Should().BeNull();
        profile.JobTitle.Should().BeNull();
        profile.Status.Should().BeNull();
        profile.Username.Should().BeNull();
        profile.EmployeeId.Should().BeNull();
        profile.ParentEmail.Should().BeNull();
        profile.ParentId.Should().BeNull();
        profile.ParentPersonId.Should().BeNull();
    }

    [Fact]
    public void Empty_string_or_whitespace_observation_is_treated_as_null()
    {
        var obs = new[]
        {
            Obs(ValueTypes.Email, "alice@example.com"),
            Obs(ValueTypes.Department, ""),
            Obs(ValueTypes.Division, "   "),
        };

        var profile = ProfileAssembler.Assemble(PersonId, TenantId, obs, Array.Empty<PersonSourceId>());

        profile.Department.Should().BeNull();
        profile.Division.Should().BeNull();
    }

    [Fact]
    public void Falls_back_to_display_name_when_first_last_absent()
    {
        var obs = new[]
        {
            Obs(ValueTypes.Email, "alice@example.com"),
            Obs(ValueTypes.DisplayName, "Smith, Alice"),
        };

        var profile = ProfileAssembler.Assemble(PersonId, TenantId, obs, Array.Empty<PersonSourceId>());

        profile.FirstName.Should().Be("Alice");
        profile.LastName.Should().Be("Smith");
    }

    [Fact]
    public void Picks_latest_value_per_type()
    {
        var obs = new[]
        {
            Obs(ValueTypes.Email, "old@example.com", createdAt: new DateTime(2025, 1, 1, 0, 0, 0, DateTimeKind.Utc)),
            Obs(ValueTypes.Email, "new@example.com", createdAt: new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc)),
        };

        var profile = ProfileAssembler.Assemble(PersonId, TenantId, obs, Array.Empty<PersonSourceId>());

        profile.Email.Should().Be("new@example.com");
    }

    [Fact]
    public void Surfaces_parent_attributes_when_present()
    {
        var parentGuid = Guid.NewGuid();
        var obs = new[]
        {
            Obs(ValueTypes.Email, "alice@example.com"),
            Obs(ValueTypes.ParentEmail, "boss@example.com"),
            Obs(ValueTypes.ParentId, "BOSS-001"),
            Obs(ValueTypes.ParentPersonId, parentGuid.ToString("D")),
        };

        var profile = ProfileAssembler.Assemble(PersonId, TenantId, obs, Array.Empty<PersonSourceId>());

        profile.ParentEmail.Should().Be("boss@example.com");
        profile.ParentId.Should().Be("BOSS-001");
        profile.ParentPersonId.Should().Be(parentGuid);
    }

    [Fact]
    public void Empty_ids_list_is_preserved_not_synthesised()
    {
        var obs = new[] { Obs(ValueTypes.Email, "alice@x.com") };
        var profile = ProfileAssembler.Assemble(PersonId, TenantId, obs, Array.Empty<PersonSourceId>());
        profile.Ids.Should().BeEmpty();
    }
}
