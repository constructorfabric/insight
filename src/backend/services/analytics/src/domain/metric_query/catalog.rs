//! The definitions this endpoint answers from, resolved once per process.
//!
//! Both inputs are compiled into the binary, so a failure here is an authoring
//! error in this release rather than a question about the environment.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::domain::compiler::error::CompileError;
use crate::domain::compiler::group_ranking::compile_group_ranking_query;
use crate::domain::compiler::metric::compile_metric_query;
use crate::domain::compiler::request::{GroupRankingQuery, MetricQuery};
use crate::domain::compiler::sql::CompiledMeasureQuery;
use crate::domain::definitions::definition::{MeasureDefinition, MetricDefinition};
use crate::domain::definitions::seeds::{SeedError, product_definitions};
use crate::domain::field_catalog::model::FieldCatalog;
use crate::domain::field_catalog::product_catalog;

/// Everything answering a question needs: the metrics, the measures they
/// compose, and the catalog both resolve their fields against.
pub struct MetricCatalog {
    catalog: &'static FieldCatalog,
    measures: BTreeMap<String, MeasureDefinition>,
    metrics: BTreeMap<String, MetricDefinition>,
}

impl std::fmt::Debug for MetricCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricCatalog")
            .field("datasets", &self.catalog.datasets.len())
            .field("measures", &self.measures.len())
            .field("metrics", &self.metrics.len())
            .finish()
    }
}

/// The shipped definitions, parsed and validated once.
pub fn product_metric_catalog() -> Result<&'static MetricCatalog, &'static SeedError> {
    static CATALOG: OnceLock<Result<MetricCatalog, SeedError>> = OnceLock::new();
    CATALOG.get_or_init(MetricCatalog::load).as_ref()
}

impl MetricCatalog {
    fn load() -> Result<Self, SeedError> {
        let catalog = product_catalog().map_err(|error| SeedError::Catalog(error.to_string()))?;
        let definitions = product_definitions()?;

        let loaded = Self {
            catalog,
            measures: definitions
                .measures
                .into_iter()
                .map(|measure| (measure.key.clone(), measure))
                .collect(),
            metrics: definitions
                .metrics
                .into_iter()
                .map(|metric| (metric.key.clone(), metric))
                .collect(),
        };

        tracing::info!(
            metrics = loaded.metrics.len(),
            measures = loaded.measures.len(),
            "metric values loaded the shipped definitions"
        );
        Ok(loaded)
    }

    #[must_use]
    pub fn metric(&self, metric_key: &str) -> Option<&MetricDefinition> {
        self.metrics.get(metric_key)
    }

    pub(super) fn compile(
        &self,
        metric: &MetricDefinition,
        query: &MetricQuery,
    ) -> Result<CompiledMeasureQuery, CompileError> {
        compile_metric_query(self.catalog, metric, &self.measures, query)
    }

    /// The pre-pass ranking a split's groups, over a metric that need not be
    /// the one the capped question reads.
    pub(super) fn compile_ranking(
        &self,
        metric: &MetricDefinition,
        query: &GroupRankingQuery,
    ) -> Result<CompiledMeasureQuery, CompileError> {
        compile_group_ranking_query(self.catalog, metric, &self.measures, query)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_definitions_load_and_carry_the_metrics_they_declare() {
        let catalog = product_metric_catalog().expect("the shipped definitions load");

        assert!(catalog.metric("git.commits").is_some());
        assert!(catalog.metric("git.not_a_shipped_metric").is_none());
    }

    #[test]
    fn the_catalog_is_resolved_once_and_shared() {
        let first = product_metric_catalog().expect("loads");
        let second = product_metric_catalog().expect("loads");

        assert!(std::ptr::eq(first, second));
    }
}
