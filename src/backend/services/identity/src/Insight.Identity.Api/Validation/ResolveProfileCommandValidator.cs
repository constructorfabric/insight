using FluentValidation;
using Insight.Identity.Api.Contracts;

namespace Insight.Identity.Api.Validation;

/// <summary>
/// FluentValidation rules for the <c>POST /v1/profiles</c> body. Two valid
/// shapes selected by <c>value_type</c>; cross-field constraints expressed
/// via <c>When</c>.
/// </summary>
public sealed class ResolveProfileCommandValidator : AbstractValidator<ResolveProfileCommandModel>
{
    public ResolveProfileCommandValidator()
    {
        RuleFor(x => x.ValueType)
            .NotEmpty()
            .WithErrorCode("urn:insight:error:invalid_value_type")
            .WithMessage("value_type is required")
            .Must(v => v is "email" or "id")
            .WithErrorCode("urn:insight:error:invalid_value_type")
            .WithMessage("value_type must be 'email' or 'id'");

        RuleFor(x => x.Value)
            .NotEmpty()
            .WithErrorCode("urn:insight:error:invalid_value")
            .WithMessage("value is required")
            .MaximumLength(320)
            .WithErrorCode("urn:insight:error:invalid_value")
            .WithMessage("value must be at most 320 characters (RFC 5321/5322)");

        When(x => x.ValueType == "id", () =>
        {
            RuleFor(x => x.InsightSourceType)
                .NotEmpty()
                .WithErrorCode("urn:insight:error:missing_source_for_id")
                .WithMessage("insight_source_type is required when value_type='id'");

            RuleFor(x => x.InsightSourceId)
                .NotNull()
                .WithErrorCode("urn:insight:error:missing_source_for_id")
                .WithMessage("insight_source_id is required when value_type='id'");
        });

        When(x => x.ValueType == "email", () =>
        {
            RuleFor(x => x.InsightSourceType)
                .Null()
                .WithErrorCode("urn:insight:error:source_not_allowed_for_email")
                .WithMessage("insight_source_type must not be set when value_type='email'");

            RuleFor(x => x.InsightSourceId)
                .Null()
                .WithErrorCode("urn:insight:error:source_not_allowed_for_email")
                .WithMessage("insight_source_id must not be set when value_type='email'");
        });
    }
}
