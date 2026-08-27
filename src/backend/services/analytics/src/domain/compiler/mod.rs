//! Turns a definition plus a query request into ClickHouse SQL.
//!
//! The compiler generates statements and nothing else: it returns the SQL text
//! and the parameters to bind against it, and never touches a connection.

// The measure-level read is reached from tests until a caller above asks for one.
#![allow(dead_code)]

mod bins;
mod combined_split;
pub mod comparison;
pub(crate) mod dimensions;
pub mod drilldown;
pub mod error;
#[cfg(test)]
mod fixtures;
mod fold;
mod group_cap;
pub mod group_ranking;
pub mod measure;
pub mod metric;
mod pool;
mod quantiles;
pub mod request;
pub mod sql;
mod subject_series;
mod subject_split;
#[cfg(test)]
mod test_catalog;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod product_tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU32;

    use chrono::NaiveDate;

    use crate::domain::definitions::definition::{
        Computation, MeasureDefinition, MetricDefinition,
    };
    use crate::domain::definitions::seeds::product_definitions;
    use crate::domain::field_catalog::model::FieldCatalog;
    use crate::domain::field_catalog::product_catalog;

    use super::drilldown::compile_drilldown;
    use super::error::CompileError;
    use super::group_ranking::compile_group_ranking_query;
    use super::metric::compile_metric_query;
    use super::request::{
        BinsView, Bucket, CombinedSplitView, ComparisonPopulation, ComparisonView, DrilldownCursor,
        DrilldownQuery, EntityScope, GroupLimit, GroupRankingQuery, MetricQuery, QuantilesView,
        RankedDimension, RankedGroup, ResolvedPerson, SubjectSeriesView, SubjectSplitView,
        ViewKind,
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
        /// The measure the one scan is grained by, which every other input
        /// agrees with on dataset, entity and event time.
        fn grain(&self, metric: &MetricDefinition) -> &MeasureDefinition {
            let key = *metric
                .input_measures()
                .first()
                .expect("a shipped metric composes at least one measure");
            self.measures
                .get(key)
                .expect("a shipped metric reads a shipped measure")
        }

        /// INVARIANT: a metric's dimension capability is the intersection of
        /// its inputs' sets, so a view may only name a key all of them declare.
        fn shared_dimensions(&self, metric: &MetricDefinition) -> Vec<String> {
            let mut shared: Option<Vec<String>> = None;
            for key in metric.input_measures() {
                let declared: Vec<String> = self
                    .measures
                    .get(key)
                    .expect("a shipped metric reads a shipped measure")
                    .dimensions
                    .iter()
                    .map(|binding| binding.key.clone())
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

        fn compile(
            &self,
            metric: &MetricDefinition,
            view: ViewKind,
        ) -> Result<CompiledMeasureQuery, CompileError> {
            self.compile_scoped(metric, EntityScope::Tenant, view)
        }

        fn compile_scoped(
            &self,
            metric: &MetricDefinition,
            entity_scope: EntityScope,
            view: ViewKind,
        ) -> Result<CompiledMeasureQuery, CompileError> {
            compile_metric_query(
                self.catalog,
                metric,
                &self.measures,
                &query(entity_scope, view),
            )
        }
    }

    fn scopes() -> [EntityScope; 2] {
        [
            EntityScope::Tenant,
            EntityScope::People(vec![
                ResolvedPerson {
                    person_ref: "person-1".to_owned(),
                    identities: vec![
                        "one@example.com".to_owned(),
                        "one.alt@example.com".to_owned(),
                    ],
                },
                ResolvedPerson {
                    person_ref: "person-2".to_owned(),
                    identities: vec!["two@example.com".to_owned()],
                },
            ]),
        ]
    }

    fn scope_name(scope: &EntityScope) -> &'static str {
        match scope {
            EntityScope::Tenant => "a tenant scope",
            EntityScope::Identities(_) => "an identity scope",
            EntityScope::People(_) => "a people scope",
        }
    }

    fn query(entity_scope: EntityScope, view: ViewKind) -> MetricQuery {
        MetricQuery {
            tenant_id: "tenant".to_owned(),
            entity_scope,
            from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            bucket: Bucket::Week,
            dimension_filters: Vec::new(),
            view,
            row_limit: 10_000,
        }
    }

    fn comparison_view(cohort_key: &str) -> ViewKind {
        ViewKind::Comparison(ComparisonView {
            population: ComparisonPopulation::DeclaredCohort {
                cohort_key: cohort_key.to_owned(),
            },
            targets: vec!["person-1".to_owned()],
            pool: vec![
                ResolvedPerson {
                    person_ref: "person-1".to_owned(),
                    identities: vec!["one@example.com".to_owned()],
                },
                ResolvedPerson {
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

    fn subject_series(dimensions: Vec<String>, group_limit: Option<GroupLimit>) -> ViewKind {
        ViewKind::SubjectSeries(SubjectSeriesView {
            dimensions,
            group_limit,
        })
    }

    fn bins_view(bins: u32) -> ViewKind {
        ViewKind::Bins(BinsView {
            bins: NonZeroU32::new(bins).expect("a bins read cuts at least one bin"),
        })
    }

    fn quantiles() -> ViewKind {
        ViewKind::Quantiles(QuantilesView {
            quantiles: vec![0.1, 0.5, 0.9],
        })
    }

    /// The computations taken over a measure's own per-row values, which are
    /// the only ones a bins read cuts or a quantile read ranks.
    fn binnable(metric: &MetricDefinition) -> bool {
        matches!(
            metric.computation,
            Computation::Percentile { .. } | Computation::Stddev { .. }
        )
    }

    fn supported_views(shipped: &Shipped, metric: &MetricDefinition) -> Vec<ViewKind> {
        let mut views = vec![ViewKind::SubjectTotal, subject_series(Vec::new(), None)];

        if let Some(key) = shipped.shared_dimensions(metric).first() {
            let dimensions = vec![key.clone()];
            views.push(subject_series(dimensions.clone(), None));
            views.push(subject_series(dimensions.clone(), Some(group_cap())));
            views.push(ViewKind::SubjectSplit(SubjectSplitView {
                dimensions: dimensions.clone(),
            }));
            views.push(ViewKind::CombinedSplit(CombinedSplitView {
                dimensions: dimensions.clone(),
                group_limit: None,
            }));
            views.push(ViewKind::CombinedSplit(CombinedSplitView {
                group_limit: Some(group_cap()),
                dimensions,
            }));
        }
        if binnable(metric) {
            views.push(bins_view(10));
            views.push(bins_view(1));
            views.push(quantiles());
        }
        if let Some(cohort_key) = &metric.cohort_key {
            views.push(comparison_view(cohort_key));
        }

        views
    }

    #[test]
    fn every_shipped_metric_compiles_in_every_view_it_supports_under_every_scope() {
        let shipped = shipped();

        for metric in &shipped.metrics {
            for scope in scopes() {
                let scope_name = scope_name(&scope);
                for view in supported_views(&shipped, metric) {
                    let name = view.name();
                    let compiled = shipped.compile_scoped(metric, scope.clone(), view);

                    assert!(
                        compiled.is_ok(),
                        "metric `{}` must compile for {name} under {scope_name}: {}",
                        metric.key,
                        compiled
                            .err()
                            .map(|error| error.to_string())
                            .unwrap_or_default()
                    );
                }
            }
        }
    }

    #[test]
    fn every_compiled_view_binds_one_parameter_per_placeholder() {
        let shipped = shipped();

        for metric in &shipped.metrics {
            for scope in scopes() {
                let scope_name = scope_name(&scope);
                for view in supported_views(&shipped, metric) {
                    let name = view.name();
                    let compiled = shipped
                        .compile_scoped(metric, scope.clone(), view)
                        .expect("compiles");

                    assert_eq!(
                        compiled.sql.matches('?').count(),
                        compiled.params.len(),
                        "metric `{}` in {name} under {scope_name}",
                        metric.key
                    );
                }
            }
        }
    }

    #[test]
    fn every_people_scoped_view_reaches_its_entities_through_the_pool_it_declares() {
        let shipped = shipped();
        let [_, people] = scopes();

        for metric in &shipped.metrics {
            for view in supported_views(&shipped, metric) {
                if matches!(view, ViewKind::Comparison(_)) {
                    continue;
                }
                let name = view.name();
                let compiled = shipped
                    .compile_scoped(metric, people.clone(), view)
                    .expect("compiles");

                assert!(
                    compiled.sql.contains("INNER JOIN pool ON pool.identity = "),
                    "metric `{}` in {name}: {}",
                    metric.key,
                    compiled.sql
                );
            }
        }
    }

    #[test]
    fn only_a_metric_over_per_row_values_has_a_distribution() {
        let shipped = shipped();

        for (view, name) in [(bins_view(10), "bins"), (quantiles(), "quantiles")] {
            for metric in &shipped.metrics {
                let compiled = shipped.compile(metric, view.clone());

                if binnable(metric) {
                    assert!(compiled.is_ok(), "metric `{}` in {name}", metric.key);
                } else {
                    assert!(
                        matches!(compiled, Err(CompileError::UnsupportedView { view, .. }) if view == name),
                        "metric `{}` in {name}: {compiled:?}",
                        metric.key
                    );
                }
            }
        }
    }

    #[test]
    fn a_comparison_read_names_the_cohort_the_metric_declares() {
        let shipped = shipped();

        for metric in &shipped.metrics {
            let compiled = shipped.compile(metric, comparison_view("not_a_declared_cohort"));

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
                subject_series(dimensions.clone(), None),
                subject_series(dimensions.clone(), Some(group_cap())),
                ViewKind::SubjectSplit(SubjectSplitView {
                    dimensions: dimensions.clone(),
                }),
                ViewKind::CombinedSplit(CombinedSplitView {
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

    fn drilldown(entity_scope: EntityScope) -> DrilldownQuery {
        DrilldownQuery {
            tenant_id: "tenant".to_owned(),
            entity_scope,
            from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            dimension_filters: Vec::new(),
            display_dimensions: Vec::new(),
            page_size: 50,
            cursor: None,
        }
    }

    #[test]
    fn every_shipped_metric_pages_the_rows_its_value_was_read_from() {
        let shipped = shipped();

        for metric in &shipped.metrics {
            for scope in scopes() {
                let scope_name = scope_name(&scope);
                let compiled = compile_drilldown(
                    shipped.catalog,
                    metric,
                    &shipped.measures,
                    &drilldown(scope),
                );

                let pages = compiled.unwrap_or_else(|error| {
                    panic!(
                        "metric `{}` must page under {scope_name}: {error}",
                        metric.key
                    )
                });
                let expected = metric.input_measures().len();
                assert_eq!(pages.len(), expected, "metric `{}`", metric.key);

                for page in pages {
                    assert_eq!(
                        page.sql.matches('?').count(),
                        page.params.len(),
                        "metric `{}` {} page under {scope_name}",
                        metric.key,
                        page.input_role
                    );
                }
            }
        }
    }

    #[test]
    fn every_people_scoped_page_reaches_its_rows_through_the_pool_it_declares() {
        let shipped = shipped();
        let [_, people] = scopes();

        for metric in &shipped.metrics {
            let pages = compile_drilldown(
                shipped.catalog,
                metric,
                &shipped.measures,
                &drilldown(people.clone()),
            )
            .expect("compiles");

            for page in pages {
                assert!(
                    page.sql.contains("INNER JOIN pool ON pool.identity = "),
                    "metric `{}`: {}",
                    metric.key,
                    page.sql
                );
                assert!(
                    page.sql.contains("    pool.person_ref AS entity_id,"),
                    "metric `{}`: {}",
                    metric.key,
                    page.sql
                );
            }
        }
    }

    #[test]
    fn a_page_resumes_on_as_many_values_as_the_dataset_orders_by() {
        let shipped = shipped();
        let metric = shipped
            .metrics
            .iter()
            .find(|metric| matches!(metric.computation, Computation::Direct { .. }))
            .expect("a shipped metric reads one measure directly");
        let dataset = shipped
            .catalog
            .dataset(&shipped.grain(metric).dataset)
            .expect("the grain reads a catalogued dataset");
        let mut ordered: Vec<&str> = Vec::new();
        for column in dataset.sorting_key.iter().chain(&dataset.row_identity) {
            if !ordered.contains(&column.as_str()) {
                ordered.push(column);
            }
        }
        let arity = ordered.len();

        let mut query = drilldown(EntityScope::Tenant);
        query.cursor = Some(DrilldownCursor {
            sort_values: vec!["position".to_owned(); arity],
        });

        let pages = compile_drilldown(shipped.catalog, metric, &shipped.measures, &query)
            .expect("compiles");

        assert!(
            arity > dataset.sorting_key.len(),
            "the identity a shipped dataset declares extends its sorting key"
        );
        assert!(
            pages[0]
                .sql
                .contains(&format!("> tuple({})", vec!["?"; arity].join(", "))),
            "{}",
            pages[0].sql
        );
    }

    #[test]
    fn every_shipped_metric_ranks_the_groups_of_a_dimension_it_declares() {
        let shipped = shipped();

        for metric in &shipped.metrics {
            let shared = shipped.shared_dimensions(metric);
            let Some(key) = shared.first() else {
                continue;
            };
            for scope in scopes() {
                let query = GroupRankingQuery {
                    tenant_id: "tenant".to_owned(),
                    entity_scope: scope.clone(),
                    from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                    to: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
                    dimension_filters: Vec::new(),
                    dimensions: vec![key.clone()],
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
}
