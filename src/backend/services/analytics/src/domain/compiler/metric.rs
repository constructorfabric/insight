//! Renders one metric-level read over its measures' dataset.
//!
//! A metric is a computation over measures plus a post-aggregation transform,
//! so the statement this writes is a measure read whose value column is the
//! computation and whose outer projection is the transform. Both view kinds
//! emit the row shape the metric-result builders already decode: a period read
//! emits `(entity_id, value)`, a timeseries read emits one row per bucket plus
//! the range total the same aggregation pipeline produces.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::domain::definitions::definition::{
    Computation, MeasureDefinition, MetricDefinition, Transform,
};
use crate::domain::definitions::filter::FilterTree;
use crate::domain::field_catalog::model::{CatalogDataset, FieldCatalog};

use super::error::CompileError;
use super::request::{Bucket, MetricQuery, ViewKind};
use super::sql::{
    CompiledMeasureQuery, EmptyFold, QueryParam, ReadScope, aggregate_expr, bucket_expr,
    conditional_aggregate_expr, from_clause, read_predicates, render_filter,
};

pub fn compile_metric_query(
    catalog: &FieldCatalog,
    metric: &MetricDefinition,
    measures: &BTreeMap<String, MeasureDefinition>,
    query: &MetricQuery,
) -> Result<CompiledMeasureQuery, CompileError> {
    let fold = Fold::resolve(metric, measures)?;
    let dataset =
        catalog
            .dataset(&fold.grain.dataset)
            .ok_or_else(|| CompileError::UnknownDataset {
                measure: fold.grain.key.clone(),
                dataset: fold.grain.dataset.clone(),
            })?;

    // A placeholder binds by position, and the value column is written before
    // the predicates are, so the fold's values bind before the scope's.
    let mut params = Vec::new();
    let value = fold.value_expr(metric, &mut params)?;
    let predicates = read_predicates(
        dataset,
        fold.grain,
        fold.where_filter,
        &ReadScope::of_metric(query),
        &mut params,
    )?;
    params.push(QueryParam::UInt(query.row_limit));

    let inner = match query.view {
        ViewKind::Period => period_sql(dataset, fold.grain, &value, &predicates),
        ViewKind::Timeseries => {
            timeseries_sql(dataset, fold.grain, &value, &predicates, query.bucket)
        }
    };

    Ok(CompiledMeasureQuery {
        sql: transformed(metric.transform.as_ref(), inner),
        params,
    })
}

/// A metric's computation resolved against the measures it names.
struct Fold<'a> {
    /// The measure whose entity, event time, and dimensions the read is
    /// grained by. A ratio folds two measures in one scan, and the numerator
    /// owns the grain both halves are read at.
    grain: &'a MeasureDefinition,
    /// The stored filter to scope the scan by, when one measure owns the whole
    /// read. A ratio keeps both filters in its folds instead.
    where_filter: Option<&'a FilterTree>,
    kind: FoldKind<'a>,
}

enum FoldKind<'a> {
    Aggregate(&'a MeasureDefinition),
    Ratio {
        numerator: &'a MeasureDefinition,
        denominator: &'a MeasureDefinition,
    },
    Quantile {
        measure: &'a MeasureDefinition,
        quantile: f64,
    },
}

impl<'a> Fold<'a> {
    fn resolve(
        metric: &MetricDefinition,
        measures: &'a BTreeMap<String, MeasureDefinition>,
    ) -> Result<Self, CompileError> {
        let input = |key: &str| {
            measures
                .get(key)
                .ok_or_else(|| CompileError::MeasureNotFound {
                    metric: metric.key.clone(),
                    measure: key.to_owned(),
                })
        };

        match &metric.computation {
            Computation::Direct { measure } => {
                let measure = input(measure)?;
                Ok(Self {
                    grain: measure,
                    where_filter: measure.filter.as_ref(),
                    kind: FoldKind::Aggregate(measure),
                })
            }
            Computation::Ratio {
                numerator,
                denominator,
            } => {
                let numerator = input(numerator)?;
                let denominator = input(denominator)?;
                agree_on(metric, numerator, denominator)?;
                Ok(Self {
                    grain: numerator,
                    where_filter: None,
                    kind: FoldKind::Ratio {
                        numerator,
                        denominator,
                    },
                })
            }
            Computation::Percentile { measure, quantile } => {
                let measure = input(measure)?;
                Ok(Self {
                    grain: measure,
                    where_filter: measure.filter.as_ref(),
                    kind: FoldKind::Quantile {
                        measure,
                        quantile: *quantile,
                    },
                })
            }
        }
    }

    /// The metric's served value as one aggregate expression over the scan.
    fn value_expr(
        &self,
        metric: &MetricDefinition,
        params: &mut Vec<QueryParam>,
    ) -> Result<String, CompileError> {
        let value = match self.kind {
            FoldKind::Aggregate(measure) => aggregate_expr(measure)?,
            FoldKind::Ratio {
                numerator,
                denominator,
            } => {
                // A numerator that matches no row is an unknown split, not a
                // zero; a zero denominator is an undefined ratio. Both read
                // NULL, and the builders never fill a NULL back in.
                let numerator = conditional_aggregate_expr(
                    numerator,
                    &fold_condition(numerator, params)?,
                    EmptyFold::Null,
                )?;
                let denominator = conditional_aggregate_expr(
                    denominator,
                    &fold_condition(denominator, params)?,
                    EmptyFold::Zero,
                )?;
                format!("{numerator} / nullIf({denominator}, 0)")
            }
            FoldKind::Quantile { measure, quantile } => {
                // The quantile is taken over the measure's per-row values: a
                // quantile of pre-folded aggregates is not that quantile.
                let value = measure.value_expr.as_deref().ok_or_else(|| {
                    CompileError::PercentileWithoutValue {
                        metric: metric.key.clone(),
                        measure: measure.key.clone(),
                    }
                })?;
                format!("quantileExact({quantile})({value})")
            }
        };

        Ok(format!("toFloat64({value})"))
    }
}

/// The rows one half of a ratio folds over, as an aggregate-function
/// condition. A measure with no stored filter folds over every scanned row.
fn fold_condition(
    measure: &MeasureDefinition,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    match &measure.filter {
        Some(filter) => render_filter(measure, filter, params),
        None => Ok("1".to_owned()),
    }
}

/// One scan can be grained one way only, so the two halves of a ratio must
/// read the same rows about the same subject at the same time.
fn agree_on(
    metric: &MetricDefinition,
    numerator: &MeasureDefinition,
    denominator: &MeasureDefinition,
) -> Result<(), CompileError> {
    let disagreement = if numerator.dataset != denominator.dataset {
        Some("the dataset they read")
    } else if numerator.entity != denominator.entity {
        Some("the field they identify an entity by")
    } else if numerator.event_time != denominator.event_time {
        Some("the field they take an event time from")
    } else {
        None
    };

    match disagreement {
        None => Ok(()),
        Some(aspect) => Err(CompileError::RatioInputsDisagree {
            metric: metric.key.clone(),
            numerator: numerator.key.clone(),
            denominator: denominator.key.clone(),
            aspect,
        }),
    }
}

fn period_sql(
    dataset: &CatalogDataset,
    measure: &MeasureDefinition,
    value: &str,
    predicates: &[String],
) -> String {
    let mut sql = String::from("SELECT\n");
    let _ = writeln!(sql, "    {} AS entity_id,", measure.entity);
    let _ = writeln!(sql, "    {value} AS value");
    let _ = writeln!(sql, "FROM {}", from_clause(dataset));
    let _ = writeln!(sql, "WHERE {}", predicates.join("\n  AND "));
    let _ = writeln!(sql, "GROUP BY entity_id");
    let _ = write!(sql, "LIMIT ?");
    sql
}

/// One row per entity and bucket plus, from the same pipeline, one row per
/// entity carrying the range total — the row the builders read a series total
/// from. `rank`, `remainder`, and `group_label` are the constants an uncapped
/// read reports; the row decoder expects the columns either way.
fn timeseries_sql(
    dataset: &CatalogDataset,
    measure: &MeasureDefinition,
    value: &str,
    predicates: &[String],
    bucket: Bucket,
) -> String {
    let bucket = bucket_expr(&measure.event_time, bucket);

    let mut sql = String::from("SELECT\n");
    let _ = writeln!(sql, "    {} AS entity_id,", measure.entity);
    let _ = writeln!(sql, "    toString({bucket}) AS bucket_start,");
    let _ = writeln!(sql, "    {value} AS value,");
    let _ = writeln!(sql, "    toUInt8(grouping({bucket})) AS is_total,");
    let _ = writeln!(sql, "    CAST(NULL AS Nullable(UInt32)) AS rank,");
    let _ = writeln!(sql, "    toUInt8(0) AS remainder,");
    let _ = writeln!(sql, "    CAST(NULL AS Nullable(String)) AS group_label");
    let _ = writeln!(sql, "FROM {}", from_clause(dataset));
    let _ = writeln!(sql, "WHERE {}", predicates.join("\n  AND "));
    let _ = writeln!(
        sql,
        "GROUP BY GROUPING SETS ((entity_id, {bucket}), (entity_id))"
    );
    let _ = writeln!(sql, "ORDER BY entity_id, is_total, bucket_start");
    let _ = write!(sql, "LIMIT ?");
    sql
}

/// Projects the transform over the aggregated value. The fold stays in the
/// inner statement so its placeholders bind once; the projection reads only
/// the `value` column, which the clamp guard references more than once.
fn transformed(transform: Option<&Transform>, inner: String) -> String {
    match transform {
        Some(transform) if !is_identity(transform) => {
            let value = transform_expr(transform, "value");
            format!("SELECT\n    * EXCEPT (value),\n    {value} AS value\nFROM (\n{inner}\n)")
        }
        Some(_) | None => inner,
    }
}

fn is_identity(transform: &Transform) -> bool {
    transform.multiplier.is_none()
        && transform.offset.is_none()
        && transform.clamp_min.is_none()
        && transform.clamp_max.is_none()
}

// SAFETY: ClickHouse `least`/`greatest` ignore NULL arguments (24.12+), so an
// unguarded clamp would resurrect an honest NULL as the clamp bound. The
// explicit guard keeps an unknown value unknown.
fn transform_expr(transform: &Transform, expr: &str) -> String {
    let mut out = expr.to_owned();
    if let Some(multiplier) = transform.multiplier {
        out = format!("{multiplier:?} * ({out})");
    }
    if let Some(offset) = transform.offset {
        out = format!("({offset:?} + {out})");
    }
    if transform.clamp_min.is_none() && transform.clamp_max.is_none() {
        return out;
    }

    let mut clamped = out.clone();
    if let Some(clamp_min) = transform.clamp_min {
        clamped = format!("greatest({clamp_min:?}, {clamped})");
    }
    if let Some(clamp_max) = transform.clamp_max {
        clamped = format!("least({clamp_max:?}, {clamped})");
    }

    format!("if(({out}) IS NULL, NULL, {clamped})")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::domain::compiler::request::{DimensionFilter, EntityScope};
    use crate::domain::compiler::test_catalog::catalog;
    use crate::domain::definitions::definition::{
        Aggregation, DimensionBinding, Direction, Format,
    };

    fn measure(key: &str, filter: Option<&str>) -> MeasureDefinition {
        MeasureDefinition {
            key: key.to_owned(),
            dataset: "git_pull_requests".to_owned(),
            description: None,
            filter: filter.map(|filter| serde_yaml::from_str(filter).expect("filter parses")),
            aggregation: Aggregation::Count,
            value_expr: None,
            subject_expr: None,
            event_time: "closed_on".to_owned(),
            entity: "author_email".to_owned(),
            dimensions: vec![DimensionBinding {
                key: "repository".to_owned(),
                value_field: "repo_slug".to_owned(),
                label_field: None,
            }],
        }
    }

    fn sized_measure(key: &str) -> MeasureDefinition {
        MeasureDefinition {
            aggregation: Aggregation::Sum,
            value_expr: Some("lines_added".to_owned()),
            ..measure(key, None)
        }
    }

    fn measures(defined: &[MeasureDefinition]) -> BTreeMap<String, MeasureDefinition> {
        defined
            .iter()
            .map(|measure| (measure.key.clone(), measure.clone()))
            .collect()
    }

    fn metric(computation: Computation) -> MetricDefinition {
        MetricDefinition {
            key: "git.merge_rate".to_owned(),
            computation,
            transform: None,
            format: Format::Percent,
            direction: Direction::HigherIsBetter,
            entity_type: "person".to_owned(),
            cohort_key: None,
            label: None,
            description: None,
        }
    }

    fn direct(measure: &str) -> Computation {
        Computation::Direct {
            measure: measure.to_owned(),
        }
    }

    fn ratio(numerator: &str, denominator: &str) -> Computation {
        Computation::Ratio {
            numerator: numerator.to_owned(),
            denominator: denominator.to_owned(),
        }
    }

    fn percentile(measure: &str, quantile: f64) -> Computation {
        Computation::Percentile {
            measure: measure.to_owned(),
            quantile,
        }
    }

    fn percent_of_total() -> Transform {
        Transform {
            multiplier: Some(100.0),
            offset: None,
            clamp_min: Some(0.0),
            clamp_max: Some(100.0),
        }
    }

    fn query(view: ViewKind) -> MetricQuery {
        MetricQuery {
            tenant_id: "acme-tenant".to_owned(),
            entity_scope: EntityScope::Tenant,
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
            bucket: Bucket::Day,
            dimension_filters: Vec::new(),
            view,
            row_limit: 10_001,
        }
    }

    fn text(value: &str) -> QueryParam {
        QueryParam::Text(value.to_owned())
    }

    fn lines(expected: &[&str]) -> String {
        expected.join("\n")
    }

    fn compile(
        metric: &MetricDefinition,
        defined: &[MeasureDefinition],
        query: &MetricQuery,
    ) -> CompiledMeasureQuery {
        compile_metric_query(&catalog(), metric, &measures(defined), query).expect("compiles")
    }

    fn compile_err(
        metric: &MetricDefinition,
        defined: &[MeasureDefinition],
        query: &MetricQuery,
    ) -> CompileError {
        compile_metric_query(&catalog(), metric, &measures(defined), query)
            .expect_err("expected a compile error")
    }

    #[test]
    fn a_direct_metric_folds_one_measure_per_entity_over_the_window() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let mut metric = metric(direct("prs_merged"));
        metric.transform = Some(percent_of_total());
        let mut query = query(ViewKind::Period);
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
        let mut query = query(ViewKind::Timeseries);
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
            &query(ViewKind::Period),
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
            &query(ViewKind::Period),
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
            (ViewKind::Period, "GROUP BY entity_id"),
            (
                ViewKind::Timeseries,
                "GROUP BY GROUPING SETS ((entity_id, toDate(closed_on)), (entity_id))",
            ),
        ];

        for (view, expected_group) in cases {
            let compiled = compile(
                &metric(percentile("pr_size", 0.5)),
                std::slice::from_ref(&sized),
                &query(view),
            );

            assert!(
                compiled
                    .sql
                    .contains("toFloat64(quantileExact(0.5)(lines_added)) AS value"),
                "{view:?}: {}",
                compiled.sql
            );
            assert!(compiled.sql.contains(expected_group), "{view:?}");
        }
    }

    #[test]
    fn a_period_read_carries_no_bucket_and_a_timeseries_read_carries_a_total_row() {
        let sized = sized_measure("pr_size");
        let metric = metric(direct("pr_size"));

        let period = compile(
            &metric,
            std::slice::from_ref(&sized),
            &query(ViewKind::Period),
        );
        let timeseries = compile(&metric, &[sized], &query(ViewKind::Timeseries));

        assert!(!period.sql.contains("bucket_start"));
        assert!(!period.sql.contains("is_total"));
        assert!(!period.sql.contains("GROUPING SETS"));

        for column in [
            "AS bucket_start,",
            "AS is_total,",
            "AS rank,",
            "AS remainder,",
            "AS group_label",
        ] {
            assert!(timeseries.sql.contains(column), "{column}");
        }
        assert!(
            timeseries
                .sql
                .contains("toUInt8(grouping(toDate(closed_on))) AS is_total,")
        );
        assert_eq!(period.params, timeseries.params);
    }

    #[test]
    fn an_identity_transform_projects_nothing_over_the_fold() {
        let sized = sized_measure("pr_size");
        let mut metric = metric(direct("pr_size"));
        metric.transform = Some(Transform::default());

        let compiled = compile(&metric, &[sized], &query(ViewKind::Period));

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

            let compiled = compile(&metric, &[sized], &query(ViewKind::Period));

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
        let mut query = query(ViewKind::Timeseries);
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
                &query(ViewKind::Period)
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
                    &query(ViewKind::Period)
                ),
                CompileError::RatioInputsDisagree {
                    metric: "git.merge_rate".to_owned(),
                    numerator: "prs_merged".to_owned(),
                    denominator: denominator_key,
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
                &query(ViewKind::Period)
            ),
            CompileError::PercentileWithoutValue {
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
                &query(ViewKind::Period)
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
        let mut query = query(ViewKind::Timeseries);
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
