using FluentAssertions;
using Insight.Identity.Domain;
using Insight.Identity.Domain.Services;
using Xunit;

namespace Insight.Identity.Tests.Unit;

public sealed class PersonAssemblerTests
{
    private static readonly Guid PersonId = Guid.Parse("11111111-1111-1111-1111-111111111111");
    private static readonly Guid SourceId = Guid.Parse("22222222-2222-2222-2222-222222222222");

    private static PersonObservation Obs(string valueType, string value, DateTime? createdAt = null) =>
        new(PersonId, "bamboohr", SourceId, valueType, value, createdAt ?? new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc));

    [Fact]
    public void Returns_null_for_empty_observations()
    {
        var person = PersonAssembler.Assemble(PersonId, Array.Empty<PersonObservation>(), parent: null, Array.Empty<Person>());
        person.Should().BeNull();
    }

    [Fact]
    public void Picks_latest_value_per_type()
    {
        var obs = new[]
        {
            Obs(ValueTypes.Email, "old@example.com", new DateTime(2025, 1, 1, 0, 0, 0, DateTimeKind.Utc)),
            Obs(ValueTypes.Email, "new@example.com", new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc)),
        };

        var person = PersonAssembler.Assemble(PersonId, obs, parent: null, Array.Empty<Person>());

        person.Should().NotBeNull();
        person!.Email.Should().Be("new@example.com");
    }

    [Fact]
    public void Falls_back_to_display_name_when_first_last_absent()
    {
        var obs = new[]
        {
            Obs(ValueTypes.DisplayName, "Smith, Alice"),
            Obs(ValueTypes.Email, "alice@example.com"),
        };

        var person = PersonAssembler.Assemble(PersonId, obs, parent: null, Array.Empty<Person>());

        person.Should().NotBeNull();
        person!.FirstName.Should().Be("Alice");
        person.LastName.Should().Be("Smith");
    }

    [Fact]
    public void Prefers_explicit_first_last_over_display_name_split()
    {
        var obs = new[]
        {
            Obs(ValueTypes.DisplayName, "Wrong, Person"),
            Obs(ValueTypes.FirstName, "Alice"),
            Obs(ValueTypes.LastName, "Smith"),
        };

        var person = PersonAssembler.Assemble(PersonId, obs, parent: null, Array.Empty<Person>());

        person!.FirstName.Should().Be("Alice");
        person.LastName.Should().Be("Smith");
    }

    [Fact]
    public void Parent_projection_fills_supervisor_and_legacy_fields()
    {
        var parentGuid = Guid.NewGuid();
        var obs = new[] { Obs(ValueTypes.Email, "alice@example.com") };
        var parent = new ParentProjection(
            PersonId: parentGuid,
            Email: "bob@example.com",
            DisplayName: "Jones, Bob",
            SourceNativeId: "BOB-7");

        var person = PersonAssembler.Assemble(PersonId, obs, parent, Array.Empty<Person>());

        person!.SupervisorEmail.Should().Be("bob@example.com");
        person.SupervisorName.Should().Be("Jones, Bob");
        person.ParentEmail.Should().Be("bob@example.com");
        person.ParentId.Should().Be("BOB-7");
        person.ParentPersonId.Should().Be(parentGuid);
    }

    [Fact]
    public void Null_parent_leaves_all_parent_fields_null_even_with_stale_observations()
    {
        // Stale value_type='parent_*' observations must not bleed
        // through when the org_chart edge is absent.
        var obs = new[]
        {
            Obs(ValueTypes.Email, "alice@example.com"),
            // Raw string literals: these value_types no longer
            // contribute to relationships (org_chart is source-of-truth)
            // and intentionally do not exist as ValueTypes constants.
            Obs("parent_email", "stale-from-persons@example.com"),
            Obs("parent_id", "STALE-1"),
            Obs("parent_person_id", Guid.NewGuid().ToString("D")),
        };

        var person = PersonAssembler.Assemble(PersonId, obs, parent: null, Array.Empty<Person>());

        person!.SupervisorEmail.Should().BeNull();
        person.SupervisorName.Should().BeNull();
        person.ParentEmail.Should().BeNull();
        person.ParentId.Should().BeNull();
        person.ParentPersonId.Should().BeNull();
    }

    [Fact]
    public void Empty_strings_for_missing_core_attributes()
    {
        var obs = new[] { Obs(ValueTypes.Email, "alice@example.com") };

        var person = PersonAssembler.Assemble(PersonId, obs, parent: null, Array.Empty<Person>());

        person!.DisplayName.Should().BeEmpty();
        person.Department.Should().BeEmpty();
        person.JobTitle.Should().BeEmpty();
        person.Status.Should().BeEmpty();
        person.SupervisorEmail.Should().BeNull();
        person.SupervisorName.Should().BeNull();
        person.ParentEmail.Should().BeNull();
        person.ParentPersonId.Should().BeNull();
    }
}
