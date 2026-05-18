using FluentAssertions;
using Insight.Identity.Api.Contracts;
using Insight.Identity.Api.Validation;
using Xunit;

namespace Insight.Identity.Tests.Unit;

public sealed class ResolveProfileCommandValidatorTests
{
    private static readonly Guid SourceId = Guid.Parse("33333333-3333-3333-3333-333333333333");

    private static readonly ResolveProfileCommandValidator Validator = new();

    [Fact]
    public void Valid_email_lookup_passes()
    {
        var body = new ResolveProfileCommandModel(
            ValueType: "email",
            Value: "alice@example.com",
            InsightSourceType: null,
            InsightSourceId: null);

        Validator.Validate(body).IsValid.Should().BeTrue();
    }

    [Fact]
    public void Valid_id_lookup_with_source_fields_passes()
    {
        var body = new ResolveProfileCommandModel(
            ValueType: "id",
            Value: "12345",
            InsightSourceType: "bamboohr",
            InsightSourceId: SourceId);

        Validator.Validate(body).IsValid.Should().BeTrue();
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("   ")]
    public void Missing_or_blank_value_type_is_rejected(string? valueType)
    {
        var body = new ResolveProfileCommandModel(valueType, "alice@example.com", null, null);
        var result = Validator.Validate(body);
        result.IsValid.Should().BeFalse();
        result.Errors.Should().Contain(e => e.ErrorCode == "urn:insight:error:invalid_value_type");
    }

    [Theory]
    [InlineData("EMAIL")]    // wrong case
    [InlineData("Email")]
    [InlineData("person_id")]
    [InlineData("username")]
    public void Unsupported_value_type_is_rejected(string valueType)
    {
        var body = new ResolveProfileCommandModel(valueType, "v", null, null);
        var result = Validator.Validate(body);
        result.IsValid.Should().BeFalse();
        result.Errors.Should().Contain(e => e.ErrorCode == "urn:insight:error:invalid_value_type");
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("   ")]
    public void Missing_or_blank_value_is_rejected(string? value)
    {
        var body = new ResolveProfileCommandModel("email", value, null, null);
        var result = Validator.Validate(body);
        result.IsValid.Should().BeFalse();
        result.Errors.Should().Contain(e => e.ErrorCode == "urn:insight:error:invalid_value");
    }

    [Fact]
    public void Value_longer_than_320_chars_is_rejected()
    {
        var body = new ResolveProfileCommandModel("email", new string('a', 321), null, null);
        var result = Validator.Validate(body);
        result.IsValid.Should().BeFalse();
        result.Errors.Should().Contain(e => e.ErrorCode == "urn:insight:error:invalid_value");
    }

    [Fact]
    public void Email_with_source_type_is_rejected()
    {
        var body = new ResolveProfileCommandModel("email", "alice@x.com", "bamboohr", null);
        var result = Validator.Validate(body);
        result.IsValid.Should().BeFalse();
        result.Errors.Should().Contain(e => e.ErrorCode == "urn:insight:error:source_not_allowed_for_email");
    }

    [Fact]
    public void Email_with_source_id_is_rejected()
    {
        var body = new ResolveProfileCommandModel("email", "alice@x.com", null, SourceId);
        var result = Validator.Validate(body);
        result.IsValid.Should().BeFalse();
        result.Errors.Should().Contain(e => e.ErrorCode == "urn:insight:error:source_not_allowed_for_email");
    }

    [Fact]
    public void Id_without_source_type_is_rejected()
    {
        var body = new ResolveProfileCommandModel("id", "12345", null, SourceId);
        var result = Validator.Validate(body);
        result.IsValid.Should().BeFalse();
        result.Errors.Should().Contain(e => e.ErrorCode == "urn:insight:error:missing_source_for_id");
    }

    [Fact]
    public void Id_without_source_id_is_rejected()
    {
        var body = new ResolveProfileCommandModel("id", "12345", "bamboohr", null);
        var result = Validator.Validate(body);
        result.IsValid.Should().BeFalse();
        result.Errors.Should().Contain(e => e.ErrorCode == "urn:insight:error:missing_source_for_id");
    }
}
