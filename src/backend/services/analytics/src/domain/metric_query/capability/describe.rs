//! The catalogue, derived from the definitions rather than declared beside
//! them. INVARIANT: every question advertised here is decided by the predicate
//! that validates the request, so the two cannot disagree.

use crate::domain::compiler::error::CompileError;
use crate::domain::definitions::definition::{Computation, MetricDefinition};

use super::super::catalog::MetricCatalog;
use super::super::comparisons::{Population, population};
use super::super::distributions::distributable;
use super::super::label::humanized;
use super::super::values::{offered_compare_offsets, offered_folds, offered_grains};
use super::dto::{
    CatalogComputation, CatalogDimension, CatalogMetric, ComparisonQuestions,
    DistributionQuestions, MetricCatalogResponse, MetricQuestions, RowsQuestions, ValuesQuestions,
};

/// INVARIANT: these enumerate the populations a comparison may name; a variant
/// added and not listed here is one no caller is ever told about.
const POPULATIONS: [Population; 2] = [Population::Tenant {}, Population::Cohort {}];

/// What a client needs to form a valid question about any shipped metric.
pub fn describe(catalog: &MetricCatalog) -> Result<MetricCatalogResponse, CompileError> {
    let metrics = catalog
        .metrics()
        .map(|metric| describe_metric(catalog, metric))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(MetricCatalogResponse { metrics })
}

fn describe_metric(
    catalog: &MetricCatalog,
    metric: &MetricDefinition,
) -> Result<CatalogMetric, CompileError> {
    let dimensions: Vec<CatalogDimension> = catalog
        .dimension_keys(&metric.key)
        .into_iter()
        .map(|key| CatalogDimension {
            label: humanized(key),
            key: key.to_owned(),
        })
        .collect();

    let splittable = !dimensions.is_empty();

    Ok(CatalogMetric {
        key: metric.key.clone(),
        label: metric.label.clone(),
        description: metric.description.clone(),
        format: metric.format,
        direction: metric.direction,
        entity_type: metric.entity_type.clone(),
        computation: computation_of(&metric.computation),
        cohort_key: metric.cohort_key.clone(),
        dimensions,
        questions: MetricQuestions {
            values: ValuesQuestions {
                grains: offered_grains(splittable),
                folds: offered_folds(splittable),
                compare: offered_compare_offsets(),
                split: splittable,
            },
            comparisons: ComparisonQuestions {
                populations: POPULATIONS
                    .into_iter()
                    .filter(|asked| population(metric, *asked).is_ok())
                    .collect(),
            },
            distributions: DistributionQuestions {
                admitted: distributable(catalog, &metric.key).is_ok(),
            },
            rows: RowsQuestions {
                inputs: catalog.input_roles(metric)?,
            },
        },
    })
}

const fn computation_of(computation: &Computation) -> CatalogComputation {
    match computation {
        Computation::Direct { .. } => CatalogComputation::Direct,
        Computation::Ratio { .. } => CatalogComputation::Ratio,
        Computation::Percentile { .. } => CatalogComputation::Percentile,
        Computation::Stddev { .. } => CatalogComputation::Stddev,
        Computation::Derived { .. } => CatalogComputation::Derived,
    }
}
