//! Why a definition and a request cannot be turned into a statement. Every
//! variant names a request that contradicts the measure or the dataset it
//! reads; a definition that passed write-time validation contributes none of
//! them on its own.

use crate::domain::definitions::filter::FilterError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompileError {
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
}
