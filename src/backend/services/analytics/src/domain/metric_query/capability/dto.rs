//! The wire shape of the catalogue: what a metric is, and what each kind of
//! question may name for it.
//!
//! INVARIANT: keys only — no dimension VALUE, and nothing read from a tenant's
//! data, ever reaches this document.

use serde::Serialize;

use crate::domain::definitions::definition::{Direction, Format};
use crate::domain::field_catalog::model::EntityType;

use super::super::comparisons::Population;
use super::super::values::{CompareOffset, Fold, Grain};

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct MetricCatalogResponse {
    /// Every metric the definitions carry, in key order.
    pub metrics: Vec<CatalogMetric>,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct CatalogMetric {
    /// The key a question names, such as `git.commits`.
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub format: Format,
    pub direction: Direction,
    /// What the metric's values are keyed by.
    pub entity_type: EntityType,
    pub computation: CatalogComputation,
    /// The grouping a cohort comparison reads; absent when the metric declares
    /// none, and then no cohort comparison is offered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cohort_key: Option<String>,
    pub dimensions: Vec<CatalogDimension>,
    pub questions: MetricQuestions,
}

/// How the value is computed, named without the measures it is computed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CatalogComputation {
    Direct,
    Ratio,
    Percentile,
    Stddev,
    Derived,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct CatalogDimension {
    /// What a filter, a split or a display dimension names.
    pub key: String,
    pub label: String,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct MetricQuestions {
    pub values: ValuesQuestions,
    pub comparisons: ComparisonQuestions,
    pub distributions: DistributionQuestions,
    pub rows: RowsQuestions,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ValuesQuestions {
    pub grains: Vec<Grain>,
    pub folds: Vec<Fold>,
    /// The earlier windows the same question may be set beside.
    pub compare: Vec<CompareOffset>,
    /// Whether the metric declares a dimension to break its value out by.
    pub split: bool,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ComparisonQuestions {
    /// Written as a comparison question's `population` field takes them.
    pub populations: Vec<Population>,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct DistributionQuestions {
    /// Whether the metric's computation is taken over per-row values, which is
    /// what having a distribution means.
    pub admitted: bool,
}

#[derive(Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct RowsQuestions {
    /// The parts of the computation a page of rows may be asked for.
    pub inputs: Vec<String>,
}

impl toolkit::api::api_dto::ResponseApiDto for MetricCatalogResponse {}
