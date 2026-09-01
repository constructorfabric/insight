//! The definitions this endpoint answers from, resolved once per process.
//!
//! Both inputs are compiled into the binary, so a failure here is an authoring
//! error in this release rather than a question about the environment.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::domain::compiler::drilldown::{
    CompiledDrilldown, DrilldownPageShape, compile_drilldown, drilldown_input_roles,
    drilldown_page_shapes, drilldown_reported_columns,
};
use crate::domain::compiler::error::CompileError;
use crate::domain::compiler::group_ranking::compile_group_ranking_query;
use crate::domain::compiler::metric::{check_distribution, compile_metric_query};
use crate::domain::compiler::request::{DrilldownQuery, GroupRankingQuery, MetricQuery};
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

    /// Every metric the definitions carry, in key order.
    pub(super) fn metrics(&self) -> impl Iterator<Item = &MetricDefinition> {
        self.metrics.values()
    }

    /// The dimension keys a question may name for a metric. INVARIANT: a
    /// metric's capability is derived, not declared — the intersection of its
    /// inputs' dimension sets, so no view can name a key one input cannot
    /// resolve.
    pub(super) fn dimension_keys(&self, metric_key: &str) -> Vec<&str> {
        let Some(metric) = self.metrics.get(metric_key) else {
            return Vec::new();
        };

        let mut shared: Option<Vec<&str>> = None;
        for key in metric.input_measures() {
            let Some(measure) = self.measures.get(key) else {
                return Vec::new();
            };
            let declared: Vec<&str> = measure
                .dimensions
                .iter()
                .map(|binding| binding.key.as_str())
                .collect();

            shared = Some(match shared {
                None => declared,
                Some(shared) => shared
                    .into_iter()
                    .filter(|key| declared.contains(key))
                    .collect(),
            });
        }

        shared.unwrap_or_default()
    }

    /// Whether a metric reports the shape of its own per-row values, named as
    /// the view that asked.
    pub(super) fn distribution(
        &self,
        metric: &MetricDefinition,
        view: &'static str,
    ) -> Result<(), CompileError> {
        check_distribution(metric, &self.measures, view)
    }

    pub(super) fn compile(
        &self,
        metric: &MetricDefinition,
        query: &MetricQuery,
    ) -> Result<CompiledMeasureQuery, CompileError> {
        compile_metric_query(self.catalog, metric, &self.measures, query)
    }

    /// One page per input of the metric's computation: the rows its value was
    /// folded from.
    pub(super) fn compile_drilldown(
        &self,
        metric: &MetricDefinition,
        query: &DrilldownQuery,
    ) -> Result<Vec<CompiledDrilldown>, CompileError> {
        compile_drilldown(self.catalog, metric, &self.measures, query)
    }

    /// The shape a page of each input has before any row is read: what a page
    /// nobody's rows reach still reports itself as.
    pub(super) fn drilldown_page_shapes(
        &self,
        metric: &MetricDefinition,
        display_dimensions: &[String],
    ) -> Result<BTreeMap<String, DrilldownPageShape>, CompileError> {
        drilldown_page_shapes(self.catalog, metric, &self.measures, display_dimensions)
    }

    /// The columns a page of `input_role` reports, which is exactly what a
    /// request may order that page by.
    ///
    /// INVARIANT: a role is resolved against [`Self::input_roles`] before a
    /// page is asked what it may be ordered by, and both read the same fold,
    /// so a role reaching here always names one of them.
    pub(super) fn drilldown_sortable_columns(
        &self,
        metric: &MetricDefinition,
        input_role: &str,
        display_dimensions: &[String],
    ) -> Result<Vec<String>, CompileError> {
        Ok(
            drilldown_reported_columns(self.catalog, metric, &self.measures, display_dimensions)?
                .remove(input_role)
                .unwrap_or_default(),
        )
    }

    /// The parts of a metric's computation a page may be asked for, named as
    /// the pages themselves are tagged.
    pub(super) fn input_roles(
        &self,
        metric: &MetricDefinition,
    ) -> Result<Vec<String>, CompileError> {
        drilldown_input_roles(metric, &self.measures)
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
