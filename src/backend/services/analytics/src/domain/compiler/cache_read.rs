//! Renders a metric read over the measures already materialized: the same
//! result-row shape a dataset read produces, re-folded from cached rows.
//!
//! INVARIANT: a cached row is already past its measure's filter, so the fold
//! that re-reads it applies no filter of its own — only the request's.

use std::collections::BTreeMap;

use crate::domain::definitions::definition::{Aggregation, MeasureDefinition, MetricDefinition};

use super::cache_build::{CACHE_RELATION, CacheRowKind};
use super::combined_split;
use super::dimensions::{
    CACHED_DATE, CACHED_KEYS, CACHED_VALUES, DimensionSource,
    combined_split_dimension_select_group, dimension_select_group,
};
use super::error::CompileError;
use super::fold::{Fold, FoldKind, ScopedRead, bounded_query};
use super::metric::subject_total_sql;
use super::pool::{Pool, joined_entity, only_cte, scan_clause};
use super::request::{EntityScope, MetricQuery, ViewKind};
use super::sql::{
    CompiledMeasureQuery, EmptyFold, QueryParam, ReadScope, TimeBucket, aggregate_function,
    dimension_binding, placeholders,
};
use super::subject_series;
use super::subject_split::subject_split_sql;

/// The column a cached row keys its entity by, standing where a dataset read
/// writes the measure's entity field.
const CACHED_ENTITY: &str = "entity";
const CACHED_VALUE: &str = "value";
const CACHED_SUBJECT: &str = "subject";

/// Why a cached read is not the shape a request asks for. Stated once so the
/// planner and the compiler refuse in the same words.
const CAPPED_RULE: &str =
    "a capped split ranks its groups over the dataset, and the cache serves no ranked read";
const DISTRIBUTION_RULE: &str = "the cache serves the value views, not the distribution ones";

/// What the cache holds for one input measure of a metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedInput {
    pub kind: CacheRowKind,
    pub definition_version: u32,
}

/// Whether the cache can answer a view at all, before anything is compiled.
#[must_use]
pub fn view_is_cacheable(view: &ViewKind) -> bool {
    match view {
        ViewKind::SubjectTotal | ViewKind::SubjectSplit(_) => true,
        ViewKind::SubjectSeries(view) => view.group_limit.is_none(),
        ViewKind::CombinedSplit(view) => view.group_limit.is_none(),
        ViewKind::Bins(_) | ViewKind::Quantiles(_) | ViewKind::Comparison(_) => false,
    }
}

/// INVARIANT: every input measure of the metric must appear in `cached`; a
/// metric one input is uncached for is read from the dataset whole.
pub fn compile_cached_metric_query(
    metric: &MetricDefinition,
    measures: &BTreeMap<String, MeasureDefinition>,
    cached: &BTreeMap<String, CachedInput>,
    query: &MetricQuery,
) -> Result<CompiledMeasureQuery, CompileError> {
    let fold = Fold::resolve(metric, measures)?;
    let pool = Pool::of_scope(&query.entity_scope);
    let source = DimensionSource::Cached(fold.grain);
    let read = cached_read(
        &fold,
        metric,
        cached,
        &ReadScope::of_metric(query),
        pool.as_ref(),
    )?;

    let unsupported = |view: &'static str, reason: &'static str| CompileError::UnsupportedView {
        metric: metric.key.clone(),
        view,
        reason,
    };

    match &query.view {
        ViewKind::SubjectTotal => {
            let inner = subject_total_sql(&read);
            Ok(bounded_query(
                metric.transform.as_ref(),
                read.params,
                query.row_limit,
                inner,
            ))
        }
        ViewKind::SubjectSplit(view) => {
            if view.dimensions.is_empty() {
                return Err(CompileError::EmptySelection {
                    selection: "the subject-split dimensions".to_owned(),
                });
            }
            let (select, group) = dimension_select_group(&source, &view.dimensions)?;
            let inner = subject_split_sql(&read, &select, &group);
            Ok(bounded_query(
                metric.transform.as_ref(),
                read.params,
                query.row_limit,
                inner,
            ))
        }
        ViewKind::SubjectSeries(view) => {
            if view.group_limit.is_some() {
                return Err(unsupported("subject_series", CAPPED_RULE));
            }
            let (select, group) = dimension_select_group(&source, &view.dimensions)?;
            let bucket = TimeBucket::over_column(CACHED_DATE, query.bucket);
            let inner = subject_series::uncapped_sql(&read, bucket.expr(), (&select, &group));
            Ok(bounded_query(
                metric.transform.as_ref(),
                read.params,
                query.row_limit,
                inner,
            ))
        }
        ViewKind::CombinedSplit(view) => {
            if view.group_limit.is_some() {
                return Err(unsupported("combined_split", CAPPED_RULE));
            }
            if view.dimensions.is_empty() {
                return Err(CompileError::EmptySelection {
                    selection: "the combined-split dimensions".to_owned(),
                });
            }
            let (select, group) = combined_split_dimension_select_group(&source, &view.dimensions)?;
            let inner = combined_split::uncapped_sql(&read, &select, &group);
            Ok(bounded_query(
                metric.transform.as_ref(),
                read.params,
                query.row_limit,
                inner,
            ))
        }
        ViewKind::Bins(_) => Err(unsupported("bins", DISTRIBUTION_RULE)),
        ViewKind::Quantiles(_) => Err(unsupported("quantiles", DISTRIBUTION_RULE)),
        ViewKind::Comparison(_) => Err(unsupported("peer", DISTRIBUTION_RULE)),
    }
}

// INVARIANT: placeholders bind by position, so parameters are pushed in the
// order the statement writes them: pool head, fold values, scope predicates —
// the order a dataset read binds in, because the same renderers write both.
fn cached_read(
    fold: &Fold<'_>,
    metric: &MetricDefinition,
    cached: &BTreeMap<String, CachedInput>,
    scope: &ReadScope<'_>,
    pool: Option<&Pool<'_>>,
) -> Result<ScopedRead, CompileError> {
    let mut params = Vec::new();
    let head = only_cte(pool, &mut params)?;
    let value = cached_value_expr(fold, metric, cached, &mut params)?;
    let predicates = cached_predicates(fold, cached, scope, &mut params)?;

    Ok(ScopedRead {
        head,
        scan: scan_clause(CACHE_RELATION.to_owned(), pool, CACHED_ENTITY, ""),
        entity: joined_entity(pool, CACHED_ENTITY).to_owned(),
        value,
        predicates,
        // INVARIANT: a cached row is a matched row, so every group the scan
        // admits is one an input matched.
        matched_group: None,
        params,
    })
}

/// Which of the scan's rows one fold reads.
enum FoldRows<'a> {
    /// Every row the scan admits, for a metric with one input.
    All,
    /// One input's rows, with what an empty fold reports.
    OfMeasure {
        condition: &'a str,
        empty: EmptyFold,
    },
}

fn cached_value_expr(
    fold: &Fold<'_>,
    metric: &MetricDefinition,
    cached: &BTreeMap<String, CachedInput>,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    let value = match fold.kind() {
        FoldKind::Aggregate(measure) => {
            cached_fold(measure, kind_of(cached, measure)?, &FoldRows::All)?
        }
        FoldKind::Ratio {
            numerator,
            denominator,
        } => {
            // INVARIANT: a zero denominator is an undefined ratio; a numerator
            // re-folds to what the live read reports empty — zero for counts.
            let numerator = one_input(cached, numerator, EmptyFold::Null, params)?;
            let denominator = one_input(cached, denominator, EmptyFold::Zero, params)?;
            format!("{numerator} / nullIf({denominator}, 0)")
        }
        FoldKind::Quantile { measure, quantile } => {
            // INVARIANT: a quantile of pre-folded aggregates is not that
            // quantile, so only per-event rows can answer one.
            per_event(measure, kind_of(cached, measure)?)?;
            format!("quantileExact({quantile})({CACHED_VALUE})")
        }
        FoldKind::Deviation { measure } => {
            per_event(measure, kind_of(cached, measure)?)?;
            format!("stddevSampIfOrNull({CACHED_VALUE}, {CACHED_VALUE} IS NOT NULL)")
        }
        FoldKind::Derived { inputs, expr } => {
            // INVARIANT: an input re-folds to what the live read reports for
            // an unmatched group — zero for counts, NULL for the rest.
            let mut folded = Vec::with_capacity(expr.references.len());
            for alias in &expr.references {
                let (_, measure) =
                    inputs
                        .iter()
                        .find(|(name, _)| name == alias)
                        .ok_or_else(|| CompileError::UnknownDerivedInput {
                            metric: metric.key.clone(),
                            alias: alias.clone(),
                        })?;
                folded.push(one_input(cached, measure, EmptyFold::Null, params)?);
            }

            expr.render(&folded)
                .map_err(|source| CompileError::MalformedExpr {
                    metric: metric.key.clone(),
                    source,
                })?
        }
    };

    Ok(format!("toFloat64({value})"))
}

/// One input of a composed metric, folded over the rows that name it, so both
/// halves are read from a single scan of the relation they share.
fn one_input(
    cached: &BTreeMap<String, CachedInput>,
    measure: &MeasureDefinition,
    empty: EmptyFold,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    let kind = kind_of(cached, measure)?;
    params.push(QueryParam::Text(measure.key.clone()));

    cached_fold(
        measure,
        kind,
        &FoldRows::OfMeasure {
            condition: "measure_key = ?",
            empty,
        },
    )
}

fn cached_fold(
    measure: &MeasureDefinition,
    kind: CacheRowKind,
    rows: &FoldRows<'_>,
) -> Result<String, CompileError> {
    let (function, operand) = cached_function(measure, kind)?;

    match rows {
        FoldRows::All => Ok(match operand {
            None => format!("{function}()"),
            Some(column) => format!("{function}({column})"),
        }),
        FoldRows::OfMeasure { condition, empty } => {
            let combinators = match live_empty_fold(measure.aggregation, *empty) {
                EmptyFold::Zero => "If",
                EmptyFold::Null => "IfOrNull",
            };
            Ok(match operand {
                None => format!("{function}{combinators}({condition})"),
                Some(column) => format!("{function}{combinators}({column}, {condition})"),
            })
        }
    }
}

/// What the live read reports for a group none of the measure's rows fall in,
/// which the re-fold reports too. A live `count` or `uniqExact` reports zero
/// there even under `-OrNull`, while the `sum` its cached days re-fold through
/// reports NULL.
fn live_empty_fold(aggregation: Aggregation, requested: EmptyFold) -> EmptyFold {
    match aggregation {
        Aggregation::Count | Aggregation::CountDistinct => EmptyFold::Zero,
        Aggregation::Sum | Aggregation::Avg | Aggregation::Min | Aggregation::Max => requested,
    }
}

/// The re-fold a cached row supports, as the function and the column it reads.
/// A row shape that cannot answer the measure's own fold is refused rather than
/// folded into a number the dataset would not agree with.
fn cached_function(
    measure: &MeasureDefinition,
    kind: CacheRowKind,
) -> Result<(&'static str, Option<&'static str>), CompileError> {
    let aggregation = measure.aggregation;
    let re_fold = match kind {
        // A pre-folded day re-folds under the same fold, except a count: the
        // build wrote per-day counts, so summing them counts each row once.
        CacheRowKind::Aggregate => match aggregation {
            Aggregation::Count | Aggregation::Sum => Some(("sum", Some(CACHED_VALUE))),
            Aggregation::Min | Aggregation::Max => {
                Some((aggregate_function(aggregation), Some(CACHED_VALUE)))
            }
            Aggregation::Avg | Aggregation::CountDistinct => None,
        },
        // A per-event row is the source row, so every fold reads it directly —
        // but a per-event build kept no subject to count distinctly.
        CacheRowKind::Event => match aggregation {
            Aggregation::Count => Some(("count", None)),
            Aggregation::Sum | Aggregation::Avg | Aggregation::Min | Aggregation::Max => {
                Some((aggregate_function(aggregation), Some(CACHED_VALUE)))
            }
            Aggregation::CountDistinct => None,
        },
        CacheRowKind::Subject => match aggregation {
            Aggregation::CountDistinct => {
                Some((aggregate_function(aggregation), Some(CACHED_SUBJECT)))
            }
            Aggregation::Count
            | Aggregation::Sum
            | Aggregation::Avg
            | Aggregation::Min
            | Aggregation::Max => None,
        },
    };

    re_fold.ok_or_else(|| CompileError::CachedFoldMismatch {
        measure: measure.key.clone(),
        aggregation: aggregation.as_db(),
        kind: kind.as_db(),
    })
}

fn per_event(measure: &MeasureDefinition, kind: CacheRowKind) -> Result<(), CompileError> {
    match kind {
        CacheRowKind::Event => Ok(()),
        CacheRowKind::Aggregate | CacheRowKind::Subject => Err(CompileError::CachedFoldMismatch {
            measure: measure.key.clone(),
            aggregation: measure.aggregation.as_db(),
            kind: kind.as_db(),
        }),
    }
}

fn kind_of(
    cached: &BTreeMap<String, CachedInput>,
    measure: &MeasureDefinition,
) -> Result<CacheRowKind, CompileError> {
    cached
        .get(&measure.key)
        .map(|input| input.kind)
        .ok_or_else(|| CompileError::MeasureNotCached {
            measure: measure.key.clone(),
        })
}

/// The `WHERE` predicates of a cached read, in binding order.
fn cached_predicates(
    fold: &Fold<'_>,
    cached: &BTreeMap<String, CachedInput>,
    scope: &ReadScope<'_>,
    params: &mut Vec<QueryParam>,
) -> Result<Vec<String>, CompileError> {
    // INVARIANT: tenancy leads every read, bound from the request's resolved
    // tenant and never written into the SQL.
    let mut predicates = vec!["tenant_id = ?".to_owned()];
    params.push(QueryParam::Text(scope.tenant_id.to_owned()));

    let mut inputs: Vec<(&str, CachedInput)> = Vec::new();
    for (_, measure) in fold.inputs() {
        let input = *cached
            .get(&measure.key)
            .ok_or_else(|| CompileError::MeasureNotCached {
                measure: measure.key.clone(),
            })?;
        if !inputs.iter().any(|(key, _)| *key == measure.key) {
            inputs.push((&measure.key, input));
        }
    }
    predicates.push(format!(
        "(measure_key, definition_version) IN ({})",
        vec!["(?, ?)"; inputs.len()].join(", ")
    ));
    for (key, input) in &inputs {
        params.push(QueryParam::Text((*key).to_owned()));
        params.push(QueryParam::UInt(u64::from(input.definition_version)));
    }

    // INVARIANT: a people-scoped read is narrowed by the join its `Pool`
    // declares rather than by a predicate, so no arm adds one here.
    match scope.entity_scope {
        EntityScope::Tenant | EntityScope::People(_) => {}
        EntityScope::Identities(identities) => {
            if identities.is_empty() {
                return Err(CompileError::EmptySelection {
                    selection: "the entity scope".to_owned(),
                });
            }
            predicates.push(format!(
                "{CACHED_ENTITY} IN ({})",
                placeholders(identities.len())
            ));
            params.extend(identities.iter().cloned().map(QueryParam::Text));
        }
    }

    predicates.push(format!("{CACHED_DATE} >= toDate(?)"));
    params.push(QueryParam::Text(scope.from.to_string()));
    predicates.push(format!("{CACHED_DATE} <= toDate(?)"));
    params.push(QueryParam::Text(scope.to.to_string()));

    for filter in scope.dimension_filters {
        // SAFETY: the key is written into the statement only after the grain
        // measure resolved it, so what lands here is an authored key.
        dimension_binding(fold.grain, &filter.key)?;
        if filter.values.is_empty() {
            return Err(CompileError::EmptySelection {
                selection: format!("dimension filter `{}`", filter.key),
            });
        }
        let index = format!("indexOf({CACHED_KEYS}, '{}')", filter.key);
        predicates.push(format!(
            "{index} > 0 AND {CACHED_VALUES}[{index}] IN ({})",
            placeholders(filter.values.len())
        ));
        params.extend(filter.values.iter().cloned().map(QueryParam::Text));
    }

    Ok(predicates)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::domain::compiler::fixtures::{
        bins_view, derived, direct, labelled_measure, lines, measure, measures, metric, people,
        people_params, percent_of_total, percentile, plain_subject_series, query, ratio,
        sized_measure, stddev, text,
    };
    use crate::domain::compiler::request::{
        Bucket, CombinedSplitView, DimensionFilter, EntityScope, GroupLimit, RankedDimension,
        RankedGroup, SubjectSeriesView, SubjectSplitView,
    };
    use crate::domain::definitions::definition::{Aggregation, MeasureDefinition};

    const VERSION: u32 = 7;

    fn cached(defined: &[MeasureDefinition], kind: CacheRowKind) -> BTreeMap<String, CachedInput> {
        defined
            .iter()
            .map(|measure| {
                (
                    measure.key.clone(),
                    CachedInput {
                        kind,
                        definition_version: VERSION,
                    },
                )
            })
            .collect()
    }

    fn compile(
        metric: &MetricDefinition,
        defined: &[MeasureDefinition],
        kind: CacheRowKind,
        request: &MetricQuery,
    ) -> CompiledMeasureQuery {
        compile_cached_metric_query(metric, &measures(defined), &cached(defined, kind), request)
            .expect("compiles")
    }

    fn compile_err(
        metric: &MetricDefinition,
        defined: &[MeasureDefinition],
        kind: CacheRowKind,
        request: &MetricQuery,
    ) -> CompileError {
        compile_cached_metric_query(metric, &measures(defined), &cached(defined, kind), request)
            .expect_err("expected a compile error")
    }

    fn weekly_series() -> ViewKind {
        ViewKind::SubjectSeries(SubjectSeriesView {
            dimensions: Vec::new(),
            group_limit: None,
        })
    }

    fn summed(key: &str) -> MeasureDefinition {
        MeasureDefinition {
            aggregation: Aggregation::Sum,
            value_expr: Some("lines_added".to_owned()),
            ..measure(key, Some("{ field: state, op: eq, value: merged }"))
        }
    }

    fn distinct(key: &str) -> MeasureDefinition {
        MeasureDefinition {
            aggregation: Aggregation::CountDistinct,
            subject_expr: Some("pull_request_id".to_owned()),
            ..measure(key, None)
        }
    }

    #[test]
    fn a_summed_measure_re_folds_its_cached_days_over_a_weekly_series_of_the_people_asked_about() {
        let lines_merged = summed("lines_merged");
        let mut request = query(weekly_series());
        request.bucket = Bucket::Week;
        request.entity_scope = people();

        let compiled = compile(
            &metric(direct("lines_merged")),
            &[lines_merged],
            CacheRowKind::Aggregate,
            &request,
        );

        assert_eq!(
            compiled.sql,
            lines(&[
                "WITH pool AS (",
                "    SELECT",
                "        member.1 AS person_ref,",
                "        member.2 AS identity",
                "    FROM (SELECT arrayJoin([(?, ?), (?, ?), (?, ?)]) AS member)",
                ")",
                "SELECT",
                "    pool.person_ref AS entity_id,",
                "    toString(toStartOfWeek(toDate(metric_date), 1)) AS bucket_start,",
                "    toFloat64(sum(value)) AS value,",
                "    toUInt8(grouping(toStartOfWeek(toDate(metric_date), 1))) AS is_total,",
                "    CAST(NULL AS Nullable(UInt32)) AS rank,",
                "    toUInt8(0) AS remainder,",
                "    CAST(NULL AS Nullable(String)) AS group_label",
                "FROM insight.semantic_measure_cache",
                "INNER JOIN pool ON pool.identity = entity",
                "WHERE tenant_id = ?",
                "  AND (measure_key, definition_version) IN ((?, ?))",
                "  AND metric_date >= toDate(?)",
                "  AND metric_date <= toDate(?)",
                "GROUP BY GROUPING SETS ((entity_id, toStartOfWeek(toDate(metric_date), 1)), (entity_id))",
                "ORDER BY entity_id, is_total, bucket_start",
                "LIMIT ?",
            ])
        );
        assert_eq!(
            compiled.params,
            [
                people_params(),
                vec![
                    text("acme-tenant"),
                    text("lines_merged"),
                    QueryParam::UInt(u64::from(VERSION)),
                    text("2026-01-01"),
                    text("2026-01-31"),
                    QueryParam::UInt(10_001),
                ]
            ]
            .concat()
        );
    }

    #[test]
    fn the_measures_own_filter_is_never_re_applied_because_the_build_already_applied_it() {
        let compiled = compile(
            &metric(direct("lines_merged")),
            &[summed("lines_merged")],
            CacheRowKind::Aggregate,
            &query(ViewKind::SubjectTotal),
        );

        assert!(!compiled.sql.contains("state"), "{}", compiled.sql);
        assert!(!compiled.params.contains(&text("merged")));
    }

    #[test]
    fn each_row_kind_re_folds_through_the_column_its_build_wrote() {
        let cases = [
            (
                metric(direct("prs_merged")),
                measure("prs_merged", None),
                CacheRowKind::Aggregate,
                "toFloat64(sum(value)) AS value",
            ),
            (
                metric(direct("prs_counted")),
                distinct("prs_counted"),
                CacheRowKind::Subject,
                "toFloat64(uniqExact(subject)) AS value",
            ),
            (
                metric(percentile("pr_size", 0.5)),
                sized_measure("pr_size"),
                CacheRowKind::Event,
                "toFloat64(quantileExact(0.5)(value)) AS value",
            ),
            (
                metric(stddev("pr_size")),
                sized_measure("pr_size"),
                CacheRowKind::Event,
                "toFloat64(stddevSampIfOrNull(value, value IS NOT NULL)) AS value",
            ),
        ];

        for (metric, defined, kind, expected) in cases {
            let compiled = compile(&metric, &[defined], kind, &query(ViewKind::SubjectTotal));

            assert!(compiled.sql.contains(expected), "{}", compiled.sql);
        }
    }

    #[test]
    fn an_averaged_measure_reads_its_kept_rows_rather_than_averaging_averages() {
        let averaged = MeasureDefinition {
            aggregation: Aggregation::Avg,
            ..sized_measure("pr_size")
        };

        let compiled = compile(
            &metric(direct("pr_size")),
            &[averaged],
            CacheRowKind::Event,
            &query(ViewKind::SubjectTotal),
        );

        assert!(
            compiled.sql.contains("toFloat64(avg(value)) AS value"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_ratio_folds_both_inputs_in_one_scan_of_the_relation_they_share() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let closed = measure(
            "prs_closed",
            Some("{ field: state, op: in, value: [merged, closed] }"),
        );

        let compiled = compile(
            &metric(ratio("prs_merged", "prs_closed")),
            &[merged, closed],
            CacheRowKind::Aggregate,
            &query(ViewKind::SubjectTotal),
        );

        assert!(
            compiled.sql.contains(
                "toFloat64(sumIf(value, measure_key = ?) / nullIf(sumIf(value, measure_key = ?), 0)) AS value"
            ),
            "{}",
            compiled.sql
        );
        assert!(
            compiled
                .sql
                .contains("(measure_key, definition_version) IN ((?, ?), (?, ?))"),
            "{}",
            compiled.sql
        );
        assert_eq!(
            compiled.params,
            vec![
                text("prs_merged"),
                text("prs_closed"),
                text("acme-tenant"),
                text("prs_merged"),
                QueryParam::UInt(u64::from(VERSION)),
                text("prs_closed"),
                QueryParam::UInt(u64::from(VERSION)),
                text("2026-01-01"),
                text("2026-01-31"),
                QueryParam::UInt(10_001),
            ]
        );
    }

    #[test]
    fn a_derived_metric_folds_every_input_under_its_expression() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let opened = measure("prs_opened", None);
        let metric = metric(derived(
            &[("merged", "prs_merged"), ("opened", "prs_opened")],
            "(opened - merged) / opened",
        ));

        let compiled = compile(
            &metric,
            &[merged, opened],
            CacheRowKind::Aggregate,
            &query(ViewKind::SubjectTotal),
        );

        assert!(
            compiled.sql.contains(
                "toFloat64(((sumIf(value, measure_key = ?)) - (sumIf(value, measure_key = ?))) / (sumIf(value, measure_key = ?))) AS value"
            ),
            "{}",
            compiled.sql
        );
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn a_counting_input_re_folds_without_or_null_because_the_live_count_reports_zero() {
        let cases = [
            (
                CacheRowKind::Aggregate,
                measure("prs_merged", None),
                measure("prs_closed", None),
                "toFloat64(sumIf(value, measure_key = ?) / nullIf(sumIf(value, measure_key = ?), 0)) AS value",
            ),
            (
                CacheRowKind::Event,
                measure("prs_merged", None),
                measure("prs_closed", None),
                "toFloat64(countIf(measure_key = ?) / nullIf(countIf(measure_key = ?), 0)) AS value",
            ),
            (
                CacheRowKind::Subject,
                distinct("prs_counted"),
                distinct("prs_reviewed"),
                "toFloat64(uniqExactIf(subject, measure_key = ?) / nullIf(uniqExactIf(subject, measure_key = ?), 0)) AS value",
            ),
        ];

        for (kind, numerator, denominator, expected) in cases {
            let computation = ratio(&numerator.key, &denominator.key);
            let compiled = compile(
                &metric(computation),
                &[numerator, denominator],
                kind,
                &query(ViewKind::SubjectTotal),
            );

            assert!(
                compiled.sql.contains(expected),
                "{kind:?}: {}",
                compiled.sql
            );
        }
    }

    #[test]
    fn a_fold_reporting_nothing_over_no_rows_keeps_or_null_on_a_ratio_input() {
        let averaged = |key: &str| MeasureDefinition {
            aggregation: Aggregation::Avg,
            ..sized_measure(key)
        };
        let smallest = |key: &str| MeasureDefinition {
            aggregation: Aggregation::Min,
            ..sized_measure(key)
        };
        let cases = [
            (
                CacheRowKind::Aggregate,
                summed("lines_merged"),
                summed("lines_opened"),
                "sumIfOrNull(value, measure_key = ?)",
            ),
            (
                CacheRowKind::Aggregate,
                smallest("smallest_merged"),
                smallest("smallest_opened"),
                "minIfOrNull(value, measure_key = ?)",
            ),
            (
                CacheRowKind::Event,
                averaged("merged_size"),
                averaged("opened_size"),
                "avgIfOrNull(value, measure_key = ?)",
            ),
        ];

        for (kind, numerator, denominator, expected) in cases {
            let computation = ratio(&numerator.key, &denominator.key);
            let compiled = compile(
                &metric(computation),
                &[numerator, denominator],
                kind,
                &query(ViewKind::SubjectTotal),
            );

            assert!(
                compiled.sql.contains(expected),
                "{kind:?}: {}",
                compiled.sql
            );
        }
    }

    #[test]
    fn a_split_reads_each_dimension_out_of_the_tuples_the_row_carries() {
        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            CacheRowKind::Aggregate,
            &query(ViewKind::SubjectSplit(SubjectSplitView {
                dimensions: vec!["repository".to_owned(), "source".to_owned()],
            })),
        );

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    entity AS entity_id,",
                "    if(indexOf(dimensions.1, 'repository') = 0, '__unknown__', coalesce(dimensions.2[indexOf(dimensions.1, 'repository')], '__unknown__')) AS dim_0_value,",
                "    if(indexOf(dimensions.1, 'repository') = 0, 'Unknown', coalesce(dimensions.3[indexOf(dimensions.1, 'repository')], 'Unknown')) AS dim_0_label,",
                "    if(indexOf(dimensions.1, 'source') = 0, '__unknown__', coalesce(dimensions.2[indexOf(dimensions.1, 'source')], '__unknown__')) AS dim_1_value,",
                "    if(indexOf(dimensions.1, 'source') = 0, 'Unknown', coalesce(dimensions.3[indexOf(dimensions.1, 'source')], 'Unknown')) AS dim_1_label,",
                "    toFloat64(sum(value)) AS value",
                "FROM insight.semantic_measure_cache",
                "WHERE tenant_id = ?",
                "  AND (measure_key, definition_version) IN ((?, ?))",
                "  AND metric_date >= toDate(?)",
                "  AND metric_date <= toDate(?)",
                "GROUP BY entity_id, dim_0_value, dim_0_label, dim_1_value, dim_1_label",
                "ORDER BY entity_id",
                "LIMIT ?",
            ])
        );
    }

    #[test]
    fn a_combined_split_picks_each_groups_label_from_its_latest_cached_day() {
        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            CacheRowKind::Aggregate,
            &query(ViewKind::CombinedSplit(CombinedSplitView {
                dimensions: vec!["repository".to_owned()],
                group_limit: None,
            })),
        );

        assert!(
            compiled.sql.contains(
                "argMax(if(indexOf(dimensions.1, 'repository') = 0, 'Unknown', coalesce(dimensions.3[indexOf(dimensions.1, 'repository')], 'Unknown')), tuple(metric_date, if(indexOf(dimensions.1, 'repository') = 0, 'Unknown', coalesce(dimensions.3[indexOf(dimensions.1, 'repository')], 'Unknown')))) AS dim_0_label,"
            ),
            "{}",
            compiled.sql
        );
        assert!(
            compiled
                .sql
                .contains("uniqExact(entity) AS contributing_entity_count,")
        );
    }

    #[test]
    fn a_narrowing_reaches_the_cached_row_through_the_dimension_it_names() {
        let mut request = query(ViewKind::SubjectTotal);
        request.dimension_filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: vec!["example/app".to_owned(), "example/lib".to_owned()],
        }];

        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            CacheRowKind::Aggregate,
            &request,
        );

        assert!(
            compiled.sql.contains(
                "  AND indexOf(dimensions.1, 'repository') > 0 AND dimensions.2[indexOf(dimensions.1, 'repository')] IN (?, ?)"
            ),
            "{}",
            compiled.sql
        );
        assert!(compiled.params.contains(&text("example/app")));
        assert!(compiled.params.contains(&text("example/lib")));
    }

    #[test]
    fn an_identity_scope_narrows_the_cached_entity_column_by_predicate() {
        let mut request = query(ViewKind::SubjectTotal);
        request.entity_scope = EntityScope::Identities(vec!["dev@example.com".to_owned()]);

        let compiled = compile(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            CacheRowKind::Aggregate,
            &request,
        );

        assert!(
            compiled.sql.contains("  AND entity IN (?)"),
            "{}",
            compiled.sql
        );
        assert!(!compiled.sql.contains("pool"));
    }

    #[test]
    fn a_transform_projects_over_the_re_folded_value_exactly_as_a_dataset_read_does() {
        let mut metric = metric(direct("prs_merged"));
        metric.transform = Some(percent_of_total());

        let compiled = compile(
            &metric,
            &[measure("prs_merged", None)],
            CacheRowKind::Aggregate,
            &query(ViewKind::SubjectTotal),
        );

        assert!(compiled.sql.starts_with(&lines(&[
            "SELECT",
            "    * EXCEPT (value),",
            "    if((100.0 * (value)) IS NULL, NULL, least(100.0, greatest(0.0, 100.0 * (value)))) AS value",
            "FROM (",
        ])));
    }

    #[test]
    fn a_row_shape_that_cannot_answer_the_measures_fold_is_refused_rather_than_re_folded() {
        let cases = [
            (distinct("prs_counted"), CacheRowKind::Aggregate),
            (distinct("prs_counted"), CacheRowKind::Event),
            (measure("prs_merged", None), CacheRowKind::Subject),
            (
                MeasureDefinition {
                    aggregation: Aggregation::Avg,
                    ..sized_measure("pr_size")
                },
                CacheRowKind::Aggregate,
            ),
        ];

        for (defined, kind) in cases {
            let key = defined.key.clone();
            let error = compile_err(
                &metric(direct(&key)),
                &[defined],
                kind,
                &query(ViewKind::SubjectTotal),
            );

            assert!(
                matches!(error, CompileError::CachedFoldMismatch { .. }),
                "{key} as {kind:?}: {error}"
            );
        }
    }

    #[test]
    fn a_distribution_over_pre_folded_rows_is_refused_rather_than_taken() {
        for kind in [CacheRowKind::Aggregate, CacheRowKind::Subject] {
            let error = compile_err(
                &metric(percentile("pr_size", 0.5)),
                &[sized_measure("pr_size")],
                kind,
                &query(ViewKind::SubjectTotal),
            );

            assert!(
                matches!(error, CompileError::CachedFoldMismatch { .. }),
                "{kind:?}: {error}"
            );
        }
    }

    #[test]
    fn a_measure_the_gate_did_not_decide_cacheable_names_no_cached_read() {
        let error = compile_cached_metric_query(
            &metric(direct("prs_merged")),
            &measures(&[measure("prs_merged", None)]),
            &BTreeMap::new(),
            &query(ViewKind::SubjectTotal),
        )
        .expect_err("an undecided measure names no cached read");

        assert_eq!(
            error,
            CompileError::MeasureNotCached {
                measure: "prs_merged".to_owned()
            }
        );
    }

    #[test]
    fn only_the_value_views_without_a_ranked_cap_are_served_from_the_cache() {
        let capped = Some(GroupLimit {
            groups: vec![RankedGroup {
                rank: 1,
                dimensions: vec![RankedDimension {
                    value: "example/app".to_owned(),
                    label: None,
                }],
            }],
            include_remainder: true,
        });
        let cases = [
            (ViewKind::SubjectTotal, true),
            (plain_subject_series(), true),
            (
                ViewKind::SubjectSplit(SubjectSplitView {
                    dimensions: vec!["repository".to_owned()],
                }),
                true,
            ),
            (
                ViewKind::CombinedSplit(CombinedSplitView {
                    dimensions: vec!["repository".to_owned()],
                    group_limit: None,
                }),
                true,
            ),
            (
                ViewKind::SubjectSeries(SubjectSeriesView {
                    dimensions: vec!["repository".to_owned()],
                    group_limit: capped.clone(),
                }),
                false,
            ),
            (
                ViewKind::CombinedSplit(CombinedSplitView {
                    dimensions: vec!["repository".to_owned()],
                    group_limit: capped,
                }),
                false,
            ),
            (bins_view(10), false),
        ];

        for (view, expected) in cases {
            let name = view.name();
            assert_eq!(view_is_cacheable(&view), expected, "{name}");

            if !expected {
                let error = compile_err(
                    &metric(direct("prs_merged")),
                    &[labelled_measure("prs_merged")],
                    CacheRowKind::Aggregate,
                    &query(view),
                );
                assert!(
                    matches!(error, CompileError::UnsupportedView { .. }),
                    "{name}: {error}"
                );
            }
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
        let mut request = query(plain_subject_series());
        request.bucket = Bucket::Month;
        request.entity_scope = people();
        request.dimension_filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: vec!["example/app".to_owned()],
        }];

        let compiled = compile(
            &metric(ratio("prs_merged", "prs_closed")),
            &[merged, closed],
            CacheRowKind::Aggregate,
            &request,
        );

        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn no_caller_supplied_value_reaches_the_statement_text() {
        let injection = "'; DROP TABLE x; --";
        let mut request = query(ViewKind::SubjectTotal);
        request.entity_scope = EntityScope::Identities(vec![injection.to_owned()]);
        request.dimension_filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: vec![injection.to_owned()],
        }];

        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            CacheRowKind::Aggregate,
            &request,
        );

        assert!(!compiled.sql.contains("DROP TABLE"), "{}", compiled.sql);
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
