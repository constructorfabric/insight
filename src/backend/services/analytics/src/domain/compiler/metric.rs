//! Renders one metric-level read over its measures' dataset: a measure read
//! whose value column is the metric's computation and whose outer projection
//! is its post-aggregation transform. Each view kind emits its own row shape.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::domain::definitions::definition::{MeasureDefinition, MetricDefinition};
use crate::domain::field_catalog::model::FieldCatalog;

use super::error::CompileError;
use super::fold::{Fold, ScopedRead, bounded_query};
use super::pool::Pool;
use super::request::{MetricQuery, ViewKind};
use super::sql::{CompiledMeasureQuery, ReadScope};
use super::{bins, combined_split, comparison, quantiles, subject_series, subject_split};

/// Whether a metric reports the shape of its measure's own per-row values,
/// which both distribution reads need and no other view does.
pub fn check_distribution(
    metric: &MetricDefinition,
    measures: &BTreeMap<String, MeasureDefinition>,
    view: &'static str,
) -> Result<(), CompileError> {
    Fold::resolve(metric, measures)?
        .row_value_expr(metric, view)
        .map(|_| ())
}

pub fn compile_metric_query(
    catalog: &FieldCatalog,
    metric: &MetricDefinition,
    measures: &BTreeMap<String, MeasureDefinition>,
    query: &MetricQuery,
) -> Result<CompiledMeasureQuery, CompileError> {
    let fold = Fold::resolve(metric, measures)?;
    let dataset = fold.dataset(catalog)?;
    // INVARIANT: a comparison read takes its entities from its own pool, so the
    // request's scope narrows every other view and not that one.
    let pool = Pool::of_scope(&query.entity_scope);

    match &query.view {
        ViewKind::SubjectTotal => {
            let read =
                fold.scoped_read(dataset, metric, &ReadScope::of_metric(query), pool.as_ref())?;
            let inner = subject_total_sql(&read);
            Ok(bounded_query(
                metric.transform.as_ref(),
                read.params,
                query.row_limit,
                inner,
            ))
        }
        ViewKind::SubjectSeries(view) => {
            subject_series::compile(dataset, metric, &fold, query, view, pool.as_ref())
        }
        ViewKind::SubjectSplit(view) => subject_split::compile(
            dataset,
            metric,
            &fold,
            query,
            &view.dimensions,
            pool.as_ref(),
        ),
        ViewKind::CombinedSplit(view) => {
            combined_split::compile(dataset, metric, &fold, query, view, pool.as_ref())
        }
        ViewKind::Bins(view) => bins::compile(dataset, metric, &fold, query, *view, pool.as_ref()),
        ViewKind::Quantiles(view) => {
            quantiles::compile(dataset, metric, &fold, query, view, pool.as_ref())
        }
        ViewKind::Comparison(view) => comparison::compile(dataset, metric, &fold, query, view),
    }
}

fn subject_total_sql(read: &ScopedRead) -> String {
    let mut sql = read.head.clone();
    sql.push_str("SELECT\n");
    let _ = writeln!(sql, "    {} AS entity_id,", read.entity);
    let _ = writeln!(sql, "    {} AS value", read.value);
    let _ = writeln!(sql, "FROM {}", read.scan);
    let _ = writeln!(sql, "WHERE {}", read.predicates.join("\n  AND "));
    let _ = writeln!(sql, "GROUP BY entity_id");
    let _ = write!(sql, "LIMIT ?");
    sql
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::domain::compiler::fixtures::{
        bins_view, compile, compile_err, derived, direct, lines, measure, metric, percent_of_total,
        percentile, plain_subject_series, query, ratio, sized_measure, stddev, text,
    };
    use crate::domain::compiler::request::{Bucket, DimensionFilter, EntityScope};
    use crate::domain::compiler::sql::QueryParam;
    use crate::domain::definitions::definition::{Aggregation, Transform};

    #[test]
    fn a_direct_metric_folds_one_measure_per_entity_over_the_window() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let mut metric = metric(direct("prs_merged"));
        metric.transform = Some(percent_of_total());
        let mut query = query(ViewKind::SubjectTotal);
        query.entity_scope = EntityScope::Identities(vec!["dev@example.com".to_owned()]);

        let compiled = compile(&metric, &[merged], &query);

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    * EXCEPT (value),",
                "    if((100.0 * (value)) IS NULL, NULL, least(100.0, greatest(0.0, 100.0 * (value)))) AS value",
                "FROM (",
                "SELECT",
                "    author_email AS entity_id,",
                "    toFloat64(count()) AS value",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND author_email IN (?)",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "  AND state = ?",
                "GROUP BY entity_id",
                "LIMIT ?",
                ")",
            ])
        );
        assert_eq!(
            compiled.params,
            vec![
                text("acme-tenant"),
                text("dev@example.com"),
                text("2026-01-01"),
                text("2026-01-31"),
                text("merged"),
                QueryParam::UInt(10_001),
            ]
        );
    }

    #[test]
    fn a_ratio_folds_both_measures_in_one_scan() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let closed = measure(
            "prs_closed",
            Some("{ field: state, op: in, value: [merged, closed] }"),
        );
        let mut metric = metric(ratio("prs_merged", "prs_closed"));
        metric.transform = Some(percent_of_total());
        let mut query = query(plain_subject_series());
        query.bucket = Bucket::Week;

        let compiled = compile(&metric, &[merged, closed], &query);

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    * EXCEPT (value),",
                "    if((100.0 * (value)) IS NULL, NULL, least(100.0, greatest(0.0, 100.0 * (value)))) AS value",
                "FROM (",
                "SELECT",
                "    author_email AS entity_id,",
                "    toString(toStartOfWeek(toDate(closed_on), 1)) AS bucket_start,",
                "    toFloat64(countIfOrNull(state = ?) / nullIf(countIf(state IN (?, ?)), 0)) AS value,",
                "    toUInt8(grouping(toStartOfWeek(toDate(closed_on), 1))) AS is_total,",
                "    CAST(NULL AS Nullable(UInt32)) AS rank,",
                "    toUInt8(0) AS remainder,",
                "    CAST(NULL AS Nullable(String)) AS group_label",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "GROUP BY GROUPING SETS ((entity_id, toStartOfWeek(toDate(closed_on), 1)), (entity_id))",
                "ORDER BY entity_id, is_total, bucket_start",
                "LIMIT ?",
                ")",
            ])
        );
        assert_eq!(
            compiled.params,
            vec![
                text("merged"),
                text("merged"),
                text("closed"),
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                QueryParam::UInt(10_001),
            ]
        );
    }

    #[test]
    fn a_ratio_reads_null_for_an_unmatched_numerator_and_a_zero_denominator() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let opened = measure("prs_opened", None);

        let compiled = compile(
            &metric(ratio("prs_merged", "prs_opened")),
            &[merged, opened],
            &query(ViewKind::SubjectTotal),
        );

        assert!(
            compiled
                .sql
                .contains("toFloat64(countIfOrNull(state = ?) / nullIf(countIf(1), 0)) AS value"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_ratio_conditions_each_half_on_its_own_fold() {
        let merged = MeasureDefinition {
            aggregation: Aggregation::Sum,
            value_expr: Some("lines_added".to_owned()),
            ..measure(
                "lines_merged",
                Some("{ field: state, op: eq, value: merged }"),
            )
        };
        let reviewed = MeasureDefinition {
            aggregation: Aggregation::CountDistinct,
            subject_expr: Some("pull_request_id".to_owned()),
            ..measure(
                "prs_reviewed",
                Some("{ field: is_draft, op: eq, value: false }"),
            )
        };

        let compiled = compile(
            &metric(ratio("lines_merged", "prs_reviewed")),
            &[merged, reviewed],
            &query(ViewKind::SubjectTotal),
        );

        assert!(
            compiled.sql.contains(
                "toFloat64(sumIfOrNull(lines_added, state = ?) / nullIf(uniqExactIf(pull_request_id, is_draft = ?), 0)) AS value"
            ),
            "{}",
            compiled.sql
        );
        assert_eq!(compiled.params[0], text("merged"));
        assert_eq!(compiled.params[1], QueryParam::Bool(0));
    }

    #[test]
    fn a_percentile_ranks_the_measures_row_values_in_every_view() {
        let sized = sized_measure("pr_size");
        let cases = [
            (ViewKind::SubjectTotal, "GROUP BY entity_id"),
            (
                plain_subject_series(),
                "GROUP BY GROUPING SETS ((entity_id, toDate(closed_on)), (entity_id))",
            ),
        ];

        for (view, expected_group) in cases {
            let name = view.name();
            let compiled = compile(
                &metric(percentile("pr_size", 0.5)),
                std::slice::from_ref(&sized),
                &query(view),
            );

            assert!(
                compiled
                    .sql
                    .contains("toFloat64(quantileExact(0.5)(lines_added)) AS value"),
                "{name}: {}",
                compiled.sql
            );
            assert!(compiled.sql.contains(expected_group), "{name}");
        }
    }

    #[test]
    fn a_stddev_reads_the_sample_spread_of_the_measures_row_values_in_every_view() {
        let sized = sized_measure("pr_size");
        let cases = [
            (ViewKind::SubjectTotal, "GROUP BY entity_id"),
            (
                plain_subject_series(),
                "GROUP BY GROUPING SETS ((entity_id, toDate(closed_on)), (entity_id))",
            ),
        ];

        for (view, expected_group) in cases {
            let name = view.name();
            let compiled = compile(
                &metric(stddev("pr_size")),
                std::slice::from_ref(&sized),
                &query(view),
            );

            assert!(
                compiled.sql.contains(
                    "toFloat64(stddevSampIfOrNull(lines_added, lines_added IS NOT NULL)) AS value"
                ),
                "{name}: {}",
                compiled.sql
            );
            assert!(compiled.sql.contains(expected_group), "{name}");
        }
    }

    #[test]
    fn a_stddev_of_a_measure_that_folds_no_value_is_rejected() {
        assert_eq!(
            compile_err(
                &metric(stddev("prs_merged")),
                &[measure("prs_merged", None)],
                &query(ViewKind::SubjectTotal)
            ),
            CompileError::DistributionWithoutValue {
                metric: "git.merge_rate".to_owned(),
                measure: "prs_merged".to_owned(),
            }
        );
    }

    #[test]
    fn a_derived_metric_folds_every_input_in_one_scan_under_its_expression() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let opened = measure("prs_opened", None);
        let metric = metric(derived(
            &[("merged", "prs_merged"), ("opened", "prs_opened")],
            "(opened - merged) / opened",
        ));

        let compiled = compile(&metric, &[merged, opened], &query(ViewKind::SubjectTotal));

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    author_email AS entity_id,",
                "    toFloat64(((countIfOrNull(1)) - (countIfOrNull(state = ?))) / (countIfOrNull(1))) AS value",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "GROUP BY entity_id",
                "LIMIT ?",
            ])
        );
        assert_eq!(
            compiled.params,
            vec![
                text("merged"),
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                QueryParam::UInt(10_001),
            ],
            "each fold binds where the expression writes it"
        );
    }

    #[test]
    fn a_derived_metric_supports_every_view_a_ratio_does() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let opened = measure("prs_opened", None);
        let metric = metric(derived(
            &[("merged", "prs_merged"), ("opened", "prs_opened")],
            "merged - opened",
        ));

        for view in [ViewKind::SubjectTotal, plain_subject_series()] {
            let name = view.name();
            let compiled = compile(&metric, &[merged.clone(), opened.clone()], &query(view));

            assert!(
                compiled
                    .sql
                    .contains("(countIfOrNull(state = ?)) - (countIfOrNull(1))"),
                "{name}: {}",
                compiled.sql
            );
            assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
        }

        assert!(
            matches!(
                compile_err(&metric, &[merged, opened], &query(bins_view(10))),
                CompileError::UnsupportedView { view: "bins", .. }
            ),
            "a derived value is not a distribution over per-row values"
        );
    }

    #[test]
    fn a_derived_metric_over_measures_that_cannot_share_one_scan_is_rejected() {
        let merged = measure("prs_merged", None);
        let elsewhere = MeasureDefinition {
            dataset: "git_commits".to_owned(),
            event_time: "committed_on".to_owned(),
            ..measure("commits", None)
        };

        assert_eq!(
            compile_err(
                &metric(derived(
                    &[("merged", "prs_merged"), ("commits", "commits")],
                    "merged + commits",
                )),
                &[merged, elsewhere],
                &query(ViewKind::SubjectTotal)
            ),
            CompileError::InputsDisagree {
                metric: "git.merge_rate".to_owned(),
                first: "commits".to_owned(),
                other: "prs_merged".to_owned(),
                aspect: "the dataset they read",
            }
        );
    }

    #[test]
    fn a_subject_total_read_carries_no_bucket_and_a_subject_series_read_carries_a_total_row() {
        let sized = sized_measure("pr_size");
        let metric = metric(direct("pr_size"));

        let subject_total = compile(
            &metric,
            std::slice::from_ref(&sized),
            &query(ViewKind::SubjectTotal),
        );
        let subject_series = compile(&metric, &[sized], &query(plain_subject_series()));

        assert!(!subject_total.sql.contains("bucket_start"));
        assert!(!subject_total.sql.contains("is_total"));
        assert!(!subject_total.sql.contains("GROUPING SETS"));

        for column in [
            "AS bucket_start,",
            "AS is_total,",
            "AS rank,",
            "AS remainder,",
            "AS group_label",
        ] {
            assert!(subject_series.sql.contains(column), "{column}");
        }
        assert!(
            subject_series
                .sql
                .contains("toUInt8(grouping(toDate(closed_on))) AS is_total,")
        );
        assert_eq!(subject_total.params, subject_series.params);
    }

    #[test]
    fn an_identity_transform_projects_nothing_over_the_fold() {
        let sized = sized_measure("pr_size");
        let mut metric = metric(direct("pr_size"));
        metric.transform = Some(Transform::default());

        let compiled = compile(&metric, &[sized], &query(ViewKind::SubjectTotal));

        assert!(compiled.sql.starts_with("SELECT\n    author_email"));
        assert!(!compiled.sql.contains("EXCEPT"));
    }

    #[test]
    fn every_transform_field_renders_the_shape_the_clamp_guard_needs() {
        let cases = [
            (
                Transform {
                    multiplier: Some(-1.0),
                    offset: Some(100.0),
                    clamp_min: Some(0.0),
                    clamp_max: Some(100.0),
                },
                "if(((100.0 + -1.0 * (value))) IS NULL, NULL, least(100.0, greatest(0.0, (100.0 + -1.0 * (value))))) AS value",
            ),
            (
                Transform {
                    clamp_max: Some(100.0),
                    ..Transform::default()
                },
                "if((value) IS NULL, NULL, least(100.0, value)) AS value",
            ),
            (
                Transform {
                    multiplier: Some(0.5),
                    ..Transform::default()
                },
                "0.5 * (value) AS value",
            ),
        ];

        for (transform, expected) in cases {
            let sized = sized_measure("pr_size");
            let mut metric = metric(direct("pr_size"));
            metric.transform = Some(transform);

            let compiled = compile(&metric, &[sized], &query(ViewKind::SubjectTotal));

            assert!(compiled.sql.contains(expected), "{}", compiled.sql);
        }
    }

    #[test]
    fn every_placeholder_has_exactly_one_bound_parameter() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let closed = measure(
            "prs_closed",
            Some("{ field: state, op: in, value: [merged, closed] }"),
        );
        let mut query = query(plain_subject_series());
        query.bucket = Bucket::Month;
        query.entity_scope =
            EntityScope::Identities(vec!["a@example.com".to_owned(), "b@example.com".to_owned()]);
        query.dimension_filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: vec!["example/app".to_owned()],
        }];

        let compiled = compile(
            &metric(ratio("prs_merged", "prs_closed")),
            &[merged, closed],
            &query,
        );

        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
        assert_eq!(compiled.params[0], text("merged"));
        assert_eq!(compiled.params[3], text("acme-tenant"));
        assert_eq!(
            compiled.params.last(),
            Some(&QueryParam::UInt(10_001)),
            "the row limit binds last"
        );
    }

    #[test]
    fn a_metric_reading_a_measure_the_request_did_not_carry_is_rejected() {
        assert_eq!(
            compile_err(
                &metric(direct("prs_merged")),
                &[sized_measure("pr_size")],
                &query(ViewKind::SubjectTotal)
            ),
            CompileError::MeasureNotFound {
                metric: "git.merge_rate".to_owned(),
                measure: "prs_merged".to_owned(),
            }
        );
    }

    #[test]
    fn a_ratio_over_measures_that_cannot_share_one_scan_is_rejected() {
        let numerator = measure("prs_merged", None);
        let cases = [
            (
                MeasureDefinition {
                    dataset: "git_commits".to_owned(),
                    event_time: "committed_on".to_owned(),
                    ..measure("commits", None)
                },
                "the dataset they read",
            ),
            (
                MeasureDefinition {
                    entity: "repo_slug".to_owned(),
                    ..measure("prs_closed", None)
                },
                "the field they identify an entity by",
            ),
            (
                MeasureDefinition {
                    event_time: "created_on".to_owned(),
                    ..measure("prs_opened", None)
                },
                "the field they take an event time from",
            ),
        ];

        for (denominator, aspect) in cases {
            let computation = ratio("prs_merged", &denominator.key);
            let denominator_key = denominator.key.clone();

            assert_eq!(
                compile_err(
                    &metric(computation),
                    &[numerator.clone(), denominator],
                    &query(ViewKind::SubjectTotal)
                ),
                CompileError::InputsDisagree {
                    metric: "git.merge_rate".to_owned(),
                    first: "prs_merged".to_owned(),
                    other: denominator_key,
                    aspect,
                },
                "{aspect}"
            );
        }
    }

    #[test]
    fn a_percentile_of_a_measure_that_folds_no_value_is_rejected() {
        assert_eq!(
            compile_err(
                &metric(percentile("prs_merged", 0.5)),
                &[measure("prs_merged", None)],
                &query(ViewKind::SubjectTotal)
            ),
            CompileError::DistributionWithoutValue {
                metric: "git.merge_rate".to_owned(),
                measure: "prs_merged".to_owned(),
            }
        );
    }

    #[test]
    fn a_metric_over_an_uncatalogued_dataset_is_rejected() {
        let orphan = MeasureDefinition {
            dataset: "git_tags".to_owned(),
            ..sized_measure("tag_size")
        };

        assert_eq!(
            compile_err(
                &metric(direct("tag_size")),
                &[orphan],
                &query(ViewKind::SubjectTotal)
            ),
            CompileError::UnknownDataset {
                measure: "tag_size".to_owned(),
                dataset: "git_tags".to_owned(),
            }
        );
    }

    #[test]
    fn filter_values_never_reach_the_sql_text() {
        let injection = "'; DROP TABLE x; --";
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let closed = measure(
            "prs_closed",
            Some("{ field: state, op: eq, value: closed }"),
        );
        let mut query = query(plain_subject_series());
        query.entity_scope = EntityScope::Identities(vec![injection.to_owned()]);
        query.dimension_filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: vec![injection.to_owned()],
        }];

        let compiled = compile(
            &metric(ratio("prs_merged", "prs_closed")),
            &[merged, closed],
            &query,
        );

        assert!(!compiled.sql.contains("DROP TABLE"), "{}", compiled.sql);
        assert!(!compiled.sql.contains('\''), "{}", compiled.sql);
        assert_eq!(
            compiled
                .params
                .iter()
                .filter(|param| **param == text(injection))
                .count(),
            2
        );
    }
}
