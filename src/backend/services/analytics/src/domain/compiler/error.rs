//! Why a definition and a request cannot be turned into a statement. Every
//! variant names a request that contradicts the measure or the dataset it
//! reads; a definition that passed write-time validation contributes none of
//! them on its own.

use crate::domain::definitions::filter::FilterError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompileError {
    #[error("measure `{measure}` reads dataset `{dataset}`, which the catalog does not have")]
    UnknownDataset { measure: String, dataset: String },
    #[error("dataset `{dataset}` binds no tenant field")]
    NoTenantField { dataset: String },
    #[error("measure `{measure}` declares no dimension `{key}`")]
    UnknownDimension { measure: String, key: String },
    #[error("{selection} names no values, so it can never match a row")]
    EmptySelection { selection: String },
    #[error(
        "measure `{measure}` aggregates with `{aggregation}`, which needs a {operand} expression"
    )]
    MissingOperand {
        measure: String,
        aggregation: &'static str,
        operand: &'static str,
    },
    #[error("measure `{measure}` filter on `{field}` is malformed: {source}")]
    MalformedFilter {
        measure: String,
        field: String,
        #[source]
        source: FilterError,
    },
    #[error("measure `{measure}` filter on `{field}` carries the unbindable number {value}")]
    UnbindableNumber {
        measure: String,
        field: String,
        value: String,
    },
    #[error("metric `{metric}` reads measure `{measure}`, which the request did not carry")]
    MeasureNotFound { metric: String, measure: String },
    #[error(
        "metric `{metric}` divides `{numerator}` by `{denominator}`, and they disagree on {aspect}"
    )]
    RatioInputsDisagree {
        metric: String,
        numerator: String,
        denominator: String,
        aspect: &'static str,
    },
    #[error("metric `{metric}` takes a percentile of measure `{measure}`, which folds no value")]
    PercentileWithoutValue { metric: String, measure: String },
}
