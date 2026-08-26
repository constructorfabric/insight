//! Turns a definition plus a query request into ClickHouse SQL.
//!
//! The compiler generates statements and nothing else: it returns the SQL text
//! and the parameters to bind against it, and never touches a connection.
//! Execution, row decoding, and view assembly are layers above.
//!
//! A measure read emits the observation row shape the metric-result builders
//! already consume — `entity_id`, `metric_date`, `value`, and, when a
//! breakdown is asked for, `dimension_value` / `dimension_label`. A metric
//! read emits the result row shape of the view it serves, one module per view.
//! Either way a view assembles from either executor's rows.
//!
//! One read is not a view: [`ranking`] decides which dimension groups a capped
//! read keeps, and runs before the view that consumes its answer.

#![allow(dead_code)] // tests are this module's only callers in the crate

mod breakdown;
mod cap;
mod dimensions;
pub mod error;
#[cfg(test)]
mod fixtures;
mod fold;
mod histogram;
pub mod measure;
pub mod metric;
mod peer;
pub mod ranking;
pub mod request;
mod rollup;
mod sql;
#[cfg(test)]
mod test_catalog;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod product_tests {
    use std::collections::BTreeMap;

    use chrono::NaiveDate;

    use crate::domain::definitions::definition::{
        Computation, MeasureDefinition, MetricDefinition,
    };
    use crate::domain::definitions::seeds::product_definitions;
    use crate::domain::field_catalog::model::FieldCatalog;
    use crate::domain::field_catalog::product_catalog;

    use super::error::CompileError;
    use super::metric::compile_metric_query;
    use super::ranking::compile_group_ranking_query;
    use super::request::{
        BreakdownView, Bucket, EntityScope, GroupLimit, GroupRankingQuery, MetricQuery, PeerMember,
        PeerPopulation, PeerView, RankedDimension, RankedGroup, RollupView, ViewKind,
    };
    use super::sql::CompiledMeasureQuery;

    struct Shipped {
        catalog: &'static FieldCatalog,
        measures: BTreeMap<String, MeasureDefinition>,
        metrics: Vec<MetricDefinition>,
    }

    fn shipped() -> Shipped {
        let definitions = product_definitions().expect("definitions are valid");
        Shipped {
            catalog: product_catalog().expect("catalog loads"),
            measures: definitions
                .measures
                .iter()
                .map(|measure| (measure.key.clone(), measure.clone()))
                .collect(),
            metrics: definitions.metrics,
        }
    }

    impl Shipped {
        /// The measure a metric is grained by. A ratio reads both halves in one
        /// scan at the numerator's grain, so the numerator owns the dimensions.
        fn grain(&self, metric: &MetricDefinition) -> &MeasureDefinition {
            let key = match &metric.computation {
                Computation::Direct { measure } | Computation::Percentile { measure, .. } => {
                    measure
                }
                Computation::Ratio { numerator, .. } => numerator,
            };
            self.measures
                .get(key)
                .expect("a shipped metric reads a shipped measure")
        }

        fn compile(
            &self,
            metric: &MetricDefinition,
            view: ViewKind,
        ) -> Result<CompiledMeasureQuery, CompileError> {
            compile_metric_query(self.catalog, metric, &self.measures, &query(view))
        }
    }

    fn query(view: ViewKind) -> MetricQuery {
        MetricQuery {
            tenant_id: "tenant".to_owned(),
            entity_scope: EntityScope::Tenant,
            from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            bucket: Bucket::Week,
            dimension_filters: Vec::new(),
            view,
            row_limit: 10_000,
        }
    }

    fn peer_view(cohort_key: &str) -> ViewKind {
        ViewKind::Peer(PeerView {
            population: PeerPopulation::DeclaredCohort {
                cohort_key: cohort_key.to_owned(),
            },
            targets: vec!["person-1".to_owned()],
            pool: vec![
                PeerMember {
                    person_ref: "person-1".to_owned(),
                    identities: vec!["one@example.com".to_owned()],
                },
                PeerMember {
                    person_ref: "person-2".to_owned(),
                    identities: vec!["two@example.com".to_owned()],
                },
            ],
        })
    }

    fn group_cap() -> GroupLimit {
        GroupLimit {
            groups: vec![
                RankedGroup {
                    rank: 1,
                    dimensions: vec![RankedDimension {
                        value: "first".to_owned(),
                        label: Some("First".to_owned()),
                    }],
                },
                RankedGroup {
                    rank: 2,
                    dimensions: vec![RankedDimension {
                        value: "second".to_owned(),
                        label: None,
                    }],
                },
            ],
            include_remainder: true,
        }
    }

    /// Every view the metric's computation and grain admit, one request each.
    fn supported_views(shipped: &Shipped, metric: &MetricDefinition) -> Vec<ViewKind> {
        let grain = shipped.grain(metric);
        let mut views = vec![ViewKind::Period, ViewKind::Timeseries];

        if let Some(binding) = grain.dimensions.first() {
            let dimensions = vec![binding.key.clone()];
            views.push(ViewKind::Breakdown(BreakdownView {
                dimensions: dimensions.clone(),
            }));
            views.push(ViewKind::Rollup(RollupView {
                dimensions: dimensions.clone(),
                group_limit: None,
            }));
            views.push(ViewKind::Rollup(RollupView {
                group_limit: Some(group_cap()),
                dimensions,
            }));
        }
        if matches!(metric.computation, Computation::Percentile { .. }) {
            views.push(ViewKind::Histogram);
        }
        if let Some(cohort_key) = &metric.cohort_key {
            views.push(peer_view(cohort_key));
        }

        views
    }

    #[test]
    fn every_shipped_metric_compiles_in_every_view_it_supports() {
        let shipped = shipped();

        for metric in &shipped.metrics {
            for view in supported_views(&shipped, metric) {
                let name = view.name();
                let compiled = shipped.compile(metric, view);

                assert!(
                    compiled.is_ok(),
                    "metric `{}` must compile for {name}: {}",
                    metric.key,
                    compiled
                        .err()
                        .map(|error| error.to_string())
                        .unwrap_or_default()
                );
            }
        }
    }

    #[test]
    fn every_compiled_view_binds_one_parameter_per_placeholder() {
        let shipped = shipped();

        for metric in &shipped.metrics {
            for view in supported_views(&shipped, metric) {
                let name = view.name();
                let compiled = shipped.compile(metric, view).expect("compiles");

                assert_eq!(
                    compiled.sql.matches('?').count(),
                    compiled.params.len(),
                    "metric `{}` in {name}",
                    metric.key
                );
            }
        }
    }

    #[test]
    fn only_a_metric_over_per_row_values_can_be_binned() {
        let shipped = shipped();

        for metric in &shipped.metrics {
            let compiled = shipped.compile(metric, ViewKind::Histogram);

            if matches!(metric.computation, Computation::Percentile { .. }) {
                assert!(compiled.is_ok(), "metric `{}`", metric.key);
            } else {
                assert!(
                    matches!(
                        compiled,
                        Err(CompileError::UnsupportedView {
                            view: "histogram",
                            ..
                        })
                    ),
                    "metric `{}`: {compiled:?}",
                    metric.key
                );
            }
        }
    }

    #[test]
    fn a_peer_read_names_the_cohort_the_metric_declares() {
        let shipped = shipped();

        for metric in &shipped.metrics {
            let compiled = shipped.compile(metric, peer_view("not_a_declared_cohort"));

            assert!(
                matches!(compiled, Err(CompileError::UndeclaredCohort { .. })),
                "metric `{}`: {compiled:?}",
                metric.key
            );
        }
    }

    #[test]
    fn a_grouped_view_rejects_a_dimension_its_grain_does_not_declare() {
        let shipped = shipped();
        let dimensions = vec!["not_a_dimension".to_owned()];

        for metric in &shipped.metrics {
            for view in [
                ViewKind::Breakdown(BreakdownView {
                    dimensions: dimensions.clone(),
                }),
                ViewKind::Rollup(RollupView {
                    dimensions: dimensions.clone(),
                    group_limit: None,
                }),
            ] {
                let name = view.name();
                let compiled = shipped.compile(metric, view);

                assert!(
                    matches!(compiled, Err(CompileError::UnknownDimension { .. })),
                    "metric `{}` in {name}: {compiled:?}",
                    metric.key
                );
            }
        }
    }

    #[test]
    fn every_shipped_metric_ranks_the_groups_of_a_dimension_it_declares() {
        let shipped = shipped();

        for metric in &shipped.metrics {
            let Some(binding) = shipped.grain(metric).dimensions.first() else {
                continue;
            };
            let query = GroupRankingQuery {
                tenant_id: "tenant".to_owned(),
                entity_scope: EntityScope::Tenant,
                from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                to: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
                dimension_filters: Vec::new(),
                dimensions: vec![binding.key.clone()],
                count: 10,
            };

            let compiled =
                compile_group_ranking_query(shipped.catalog, metric, &shipped.measures, &query);

            assert!(
                compiled.is_ok(),
                "metric `{}`: {}",
                metric.key,
                compiled
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_default()
            );
        }
    }
}
