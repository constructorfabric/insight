//! Validating submitted definitions beside the shipped ones without keeping
//! them. INVARIANT: nothing here reaches a store — the outcome IS the answer,
//! so a set that validates is not thereby installed.

use serde::{Deserialize, Serialize};

use super::definition::{MeasureDefinition, MetricDefinition};
use super::seeds::{SeedError, product_definitions};
use super::validate::{ValidationError, validate_definitions};
use crate::domain::field_catalog;

/// Definitions to judge, in the shape the authored YAML parses into.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateDefinitionsRequest {
    #[serde(default)]
    pub measures: Vec<MeasureDefinition>,
    #[serde(default)]
    pub metrics: Vec<MetricDefinition>,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ValidateDefinitionsResponse {
    pub valid: bool,
    /// Every offender, not the first one.
    pub errors: Vec<ValidationFailure>,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ValidationFailure {
    pub kind: ValidationErrorKind,
    pub message: String,
}

/// Which rule was broken, as a discriminant a machine can branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValidationErrorKind {
    KeyShape,
    MetricKeyShape,
    DuplicateKey,
    DatasetNotFound,
    FieldNotFound,
    RoleMismatch,
    Filter,
    Expression,
    Operand,
    MeasureNotFound,
    QuantileOutOfRange,
    MixedDatasets,
    DistributionWithoutValue,
    NoDerivedInputs,
    MetricExpression,
    UnknownDerivedInput,
    UnusedDerivedInput,
    DimensionBindingsDisagree,
    EntityGrainMismatch,
    CohortWithoutPeers,
}

/// INVARIANT: the submitted definitions are judged as part of one set with the
/// shipped ones, so a submitted metric may read a shipped measure, a submitted
/// measure may be read by a shipped metric, and a key already taken collides.
pub fn dry_run(
    request: ValidateDefinitionsRequest,
) -> Result<ValidateDefinitionsResponse, SeedError> {
    let catalog =
        field_catalog::product_catalog().map_err(|error| SeedError::Catalog(error.to_string()))?;
    let shipped = product_definitions()?;

    let mut measures = shipped.measures;
    measures.extend(request.measures);
    let mut metrics = shipped.metrics;
    metrics.extend(request.metrics);

    let errors = match validate_definitions(catalog, &measures, &metrics) {
        Ok(()) => Vec::new(),
        Err(errors) => errors.iter().map(failure).collect(),
    };

    Ok(ValidateDefinitionsResponse {
        valid: errors.is_empty(),
        errors,
    })
}

fn failure(error: &ValidationError) -> ValidationFailure {
    ValidationFailure {
        kind: kind_of(error),
        message: error.to_string(),
    }
}

const fn kind_of(error: &ValidationError) -> ValidationErrorKind {
    match error {
        ValidationError::KeyShape(_) => ValidationErrorKind::KeyShape,
        ValidationError::MetricKeyShape(_) => ValidationErrorKind::MetricKeyShape,
        ValidationError::DuplicateKey(_) => ValidationErrorKind::DuplicateKey,
        ValidationError::DatasetNotFound { .. } => ValidationErrorKind::DatasetNotFound,
        ValidationError::FieldNotFound { .. } => ValidationErrorKind::FieldNotFound,
        ValidationError::RoleMismatch { .. } => ValidationErrorKind::RoleMismatch,
        ValidationError::Filter { .. } => ValidationErrorKind::Filter,
        ValidationError::Expression { .. } => ValidationErrorKind::Expression,
        ValidationError::Operand { .. } => ValidationErrorKind::Operand,
        ValidationError::MeasureNotFound { .. } => ValidationErrorKind::MeasureNotFound,
        ValidationError::QuantileOutOfRange { .. } => ValidationErrorKind::QuantileOutOfRange,
        ValidationError::MixedDatasets { .. } => ValidationErrorKind::MixedDatasets,
        ValidationError::DistributionWithoutValue { .. } => {
            ValidationErrorKind::DistributionWithoutValue
        }
        ValidationError::NoDerivedInputs { .. } => ValidationErrorKind::NoDerivedInputs,
        ValidationError::MetricExpression { .. } => ValidationErrorKind::MetricExpression,
        ValidationError::UnknownDerivedInput { .. } => ValidationErrorKind::UnknownDerivedInput,
        ValidationError::UnusedDerivedInput { .. } => ValidationErrorKind::UnusedDerivedInput,
        ValidationError::DimensionBindingsDisagree { .. } => {
            ValidationErrorKind::DimensionBindingsDisagree
        }
        ValidationError::EntityGrainMismatch { .. } => ValidationErrorKind::EntityGrainMismatch,
        ValidationError::CohortWithoutPeers { .. } => ValidationErrorKind::CohortWithoutPeers,
    }
}

impl toolkit::api::api_dto::RequestApiDto for ValidateDefinitionsRequest {}
impl toolkit::api::api_dto::ResponseApiDto for ValidateDefinitionsResponse {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
