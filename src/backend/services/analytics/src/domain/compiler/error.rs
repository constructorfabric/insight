//! Why a definition and a request cannot be turned into a statement: every
//! variant names a request that contradicts the measure or the dataset it reads.

use crate::domain::definitions::expr::ScalarExprError;
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
    #[error("metric `{metric}` composes `{first}` with `{other}`, and they disagree on {aspect}")]
    InputsDisagree {
        metric: String,
        first: String,
        other: String,
        aspect: &'static str,
    },
    #[error(
        "metric `{metric}` reads the distribution of measure `{measure}`, which folds no value"
    )]
    DistributionWithoutValue { metric: String, measure: String },
    #[error("metric `{metric}` composes no measure, so it folds nothing")]
    NoInputs { metric: String },
    #[error("metric `{metric}` expression names `{alias}`, which is not one of its inputs")]
    UnknownDerivedInput { metric: String, alias: String },
    #[error("metric `{metric}` expression is not admissible: {source}")]
    MalformedExpr {
        metric: String,
        #[source]
        source: ScalarExprError,
    },
    #[error("metric `{metric}` cannot be read as a {view} view: {reason}")]
    UnsupportedView {
        metric: String,
        view: &'static str,
        reason: &'static str,
    },
    #[error("metric `{metric}` declares no peer cohort `{cohort_key}`")]
    UndeclaredCohort { metric: String, cohort_key: String },
    #[error("dataset `{dataset}` cannot order a page of rows: {reason}")]
    UnorderableDataset { dataset: String, reason: String },
    #[error("a page resumes from {found} ordering values, and the read orders by {expected}")]
    CursorArity { expected: usize, found: usize },
    #[error("a page reports no column `{column}` to order by; it reports {sortable}")]
    UnsortableColumn { column: String, sortable: String },
    #[error(
        "the group cap ranks group {rank} by {named} dimension values, and the read groups by {requested}"
    )]
    GroupCapArity {
        rank: u32,
        named: usize,
        requested: usize,
    },
    #[error(
        "measure `{measure}` was not decided readable from the cache, so no cached read names it"
    )]
    MeasureNotCached { measure: String },
    #[error(
        "measure `{measure}` folds with `{aggregation}`, which its cached `{kind}` rows cannot answer"
    )]
    CachedFoldMismatch {
        measure: String,
        aggregation: &'static str,
        kind: &'static str,
    },
}
