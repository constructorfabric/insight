//! Renders a subject-series read: one value per entity per bucket, plus the window
//! total the same pipeline produces.
//!
//! The total is a second grouping set rather than a second statement, so a
//! series and its total are folded from the same rows. Naming dimensions
//! splits each entity into one series per dimension combination, and capping
//! them keeps the ranked groups and folds the rest into one remainder series.
//! `rank`, `remainder`, and `group_label` are the constants an uncapped read
//! reports; the row decoder expects the columns either way.

use std::fmt::Write;

use crate::domain::definitions::definition::{MeasureDefinition, MetricDefinition};
use crate::domain::field_catalog::model::CatalogDataset;

use super::dimensions::dimension_select_group;
use super::error::CompileError;
use super::fold::{Fold, ScopedRead, bounded_query, transform_in_place};
use super::group_cap::{
    CAPPED_RANK_COLUMNS, GroupCap, UNCAPPED_RANK_COLUMNS, ranked_scan_ctes, raw_dimension_select,
};
use super::request::{Bucket, MetricQuery, SubjectSeriesView};
use super::sql::{
    CompiledMeasureQuery, QueryParam, ReadScope, bucket_expr, from_clause, read_predicates,
};

pub(super) fn compile(
    dataset: &CatalogDataset,
    metric: &MetricDefinition,
    fold: &Fold<'_>,
    query: &MetricQuery,
    view: &SubjectSeriesView,
) -> Result<CompiledMeasureQuery, CompileError> {
    let Some(limit) = &view.group_limit else {
        return compile_uncapped(dataset, metric, fold, query, &view.dimensions);
    };

    if view.dimensions.is_empty() {
        return Err(CompileError::EmptySelection {
            selection: "the capped subject-series dimensions".to_owned(),
        });
    }

    let cap = GroupCap::resolve(limit, view.dimensions.len())?;
    compile_capped(dataset, metric, fold, query, &view.dimensions, &cap)
}

fn compile_uncapped(
    dataset: &CatalogDataset,
    metric: &MetricDefinition,
    fold: &Fold<'_>,
    query: &MetricQuery,
    dimensions: &[String],
) -> Result<CompiledMeasureQuery, CompileError> {
    let (select, group) = dimension_select_group(fold.grain, dimensions)?;
    let read = fold.scoped_read(dataset, metric, &ReadScope::of_metric(query))?;
    let inner = uncapped_sql(dataset, fold.grain, &read, query.bucket, (&select, &group));

    Ok(bounded_query(
        metric.transform.as_ref(),
        read.params,
        query.row_limit,
        inner,
    ))
}

/// `dimensions` is the projection of the requested dimensions and the keys
/// they group by, which an undimensioned read leaves empty.
fn uncapped_sql(
    dataset: &CatalogDataset,
    measure: &MeasureDefinition,
    read: &ScopedRead,
    bucket: Bucket,
    dimensions: (&str, &str),
) -> String {
    let bucket = bucket_expr(&measure.event_time, bucket);
    let (select, group) = dimensions;
    let (bucket_set, total_set) = if group.is_empty() {
        (format!("entity_id, {bucket}"), "entity_id".to_owned())
    } else {
        (
            format!("entity_id, {bucket}, {group}"),
            format!("entity_id, {group}"),
        )
    };

    let mut sql = String::from("SELECT\n");
    let _ = writeln!(sql, "    {} AS entity_id,", measure.entity);
    let _ = writeln!(sql, "    toString({bucket}) AS bucket_start,");
    sql.push_str(select);
    let _ = writeln!(sql, "    {} AS value,", read.value);
    let _ = writeln!(sql, "    toUInt8(grouping({bucket})) AS is_total,");
    sql.push_str(UNCAPPED_RANK_COLUMNS);
    let _ = writeln!(sql, "FROM {}", from_clause(dataset));
    let _ = writeln!(sql, "WHERE {}", read.predicates.join("\n  AND "));
    let _ = writeln!(
        sql,
        "GROUP BY GROUPING SETS (({bucket_set}), ({total_set}))"
    );
    let _ = writeln!(sql, "ORDER BY entity_id, is_total, bucket_start");
    let _ = write!(sql, "LIMIT ?");
    sql
}

/// A capped read ranks each scanned row before it folds anything, so the scan
/// is written before the fold and its values bind first. The bucket is ranked
/// alongside the dimensions because both are read per scanned row, and the
/// total is still the second grouping set the uncapped read takes it from.
fn compile_capped(
    dataset: &CatalogDataset,
    metric: &MetricDefinition,
    fold: &Fold<'_>,
    query: &MetricQuery,
    dimensions: &[String],
    cap: &GroupCap<'_>,
) -> Result<CompiledMeasureQuery, CompileError> {
    let bucket = bucket_expr(&fold.grain.event_time, query.bucket);
    let raw_dimensions = raw_dimension_select(fold.grain, dimensions)?;
    let projections = format!("        {bucket} AS bucket_start,\n{raw_dimensions}");

    let mut params = Vec::new();
    let predicates = read_predicates(
        dataset,
        fold.grain,
        fold.where_filter,
        &ReadScope::of_metric(query),
        &mut params,
    )?;
    let rank = cap.rank_expr(&mut params);
    let value = fold.value_expr(metric, &mut params)?;
    let dimension_select = cap.dimension_select(&mut params);
    params.push(QueryParam::UInt(query.row_limit));

    let mut sql = ranked_scan_ctes(
        dataset,
        &projections,
        &predicates,
        &rank,
        cap.remainder_predicate(),
    );
    let _ = writeln!(sql, "aggregated AS (");
    let _ = writeln!(sql, "    SELECT");
    let _ = writeln!(sql, "        {} AS entity_id,", fold.grain.entity);
    let _ = writeln!(sql, "        bucket_start,");
    let _ = writeln!(sql, "        group_rank,");
    let _ = writeln!(sql, "        {value} AS value,");
    let _ = writeln!(sql, "        toUInt8(grouping(bucket_start)) AS is_total");
    let _ = writeln!(sql, "    FROM filtered");
    let _ = writeln!(sql, "    GROUP BY GROUPING SETS (");
    let _ = writeln!(sql, "        (entity_id, bucket_start, group_rank),");
    let _ = writeln!(sql, "        (entity_id, group_rank)");
    let _ = writeln!(sql, "    )");
    let _ = writeln!(sql, ")");
    let _ = writeln!(sql, "SELECT");
    let _ = writeln!(sql, "    entity_id,");
    let _ = writeln!(sql, "    toString(bucket_start) AS bucket_start,");
    sql.push_str(&dimension_select);
    let _ = writeln!(
        sql,
        "    {} AS value,",
        transform_in_place(metric.transform.as_ref(), "value")
    );
    let _ = writeln!(sql, "    is_total,");
    sql.push_str(CAPPED_RANK_COLUMNS);
    let _ = writeln!(sql, "FROM aggregated");
    let _ = writeln!(
        sql,
        "ORDER BY entity_id, group_rank, is_total, bucket_start"
    );
    let _ = write!(sql, "LIMIT ?");

    Ok(CompiledMeasureQuery { sql, params })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::domain::compiler::error::CompileError;
    use crate::domain::compiler::fixtures::{
        compile, compile_err, direct, labelled_measure, lines, metric, percent_of_total,
        plain_subject_series, query, text,
    };
    use crate::domain::compiler::request::{
        DimensionFilter, GroupLimit, RankedDimension, RankedGroup, SubjectSeriesView, ViewKind,
    };
    use crate::domain::compiler::sql::QueryParam;

    fn view(dimensions: &[&str], group_limit: Option<GroupLimit>) -> ViewKind {
        ViewKind::SubjectSeries(SubjectSeriesView {
            dimensions: dimensions.iter().map(|key| (*key).to_owned()).collect(),
            group_limit,
        })
    }

    fn cap(include_remainder: bool) -> GroupLimit {
        GroupLimit {
            groups: vec![
                RankedGroup {
                    rank: 1,
                    dimensions: vec![RankedDimension {
                        value: "example/app".to_owned(),
                        label: Some("Example App".to_owned()),
                    }],
                },
                RankedGroup {
                    rank: 2,
                    dimensions: vec![RankedDimension {
                        value: "example/lib".to_owned(),
                        label: None,
                    }],
                },
            ],
            include_remainder,
        }
    }

    #[test]
    fn an_undimensioned_subject_series_reports_one_series_per_entity_beside_its_total() {
        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            &query(plain_subject_series()),
        );

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    author_email AS entity_id,",
                "    toString(toDate(closed_on)) AS bucket_start,",
                "    toFloat64(count()) AS value,",
                "    toUInt8(grouping(toDate(closed_on))) AS is_total,",
                "    CAST(NULL AS Nullable(UInt32)) AS rank,",
                "    toUInt8(0) AS remainder,",
                "    CAST(NULL AS Nullable(String)) AS group_label",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "GROUP BY GROUPING SETS ((entity_id, toDate(closed_on)), (entity_id))",
                "ORDER BY entity_id, is_total, bucket_start",
                "LIMIT ?",
            ])
        );
        assert_eq!(
            compiled.params,
            vec![
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                QueryParam::UInt(10_001),
            ]
        );
    }

    #[test]
    fn a_dimensioned_subject_series_splits_each_entity_into_one_series_per_group() {
        let mut request = query(view(&["repository", "source"], None));
        request.dimension_filters = vec![DimensionFilter {
            key: "source".to_owned(),
            values: vec!["github".to_owned()],
        }];

        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            &request,
        );

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    author_email AS entity_id,",
                "    toString(toDate(closed_on)) AS bucket_start,",
                "    coalesce(toString(repo_slug), '__unknown__') AS dim_0_value,",
                "    coalesce(toString(repo_slug), 'Unknown') AS dim_0_label,",
                "    coalesce(toString(data_source), '__unknown__') AS dim_1_value,",
                "    coalesce(toString(data_source_label), 'Unknown') AS dim_1_label,",
                "    toFloat64(count()) AS value,",
                "    toUInt8(grouping(toDate(closed_on))) AS is_total,",
                "    CAST(NULL AS Nullable(UInt32)) AS rank,",
                "    toUInt8(0) AS remainder,",
                "    CAST(NULL AS Nullable(String)) AS group_label",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "  AND data_source IN (?)",
                "GROUP BY GROUPING SETS ((entity_id, toDate(closed_on), dim_0_value, dim_0_label, dim_1_value, dim_1_label), (entity_id, dim_0_value, dim_0_label, dim_1_value, dim_1_label))",
                "ORDER BY entity_id, is_total, bucket_start",
                "LIMIT ?",
            ])
        );
        assert_eq!(
            compiled.params,
            vec![
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                text("github"),
                QueryParam::UInt(10_001),
            ]
        );
    }

    #[test]
    fn a_capped_subject_series_ranks_each_scanned_row_before_it_folds_any_bucket() {
        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            &query(view(&["repository"], Some(cap(true)))),
        );

        assert_eq!(
            compiled.sql,
            lines(&[
                "WITH scoped AS (",
                "    SELECT",
                "        *,",
                "        toDate(closed_on) AS bucket_start,",
                "        coalesce(toString(repo_slug), '__unknown__') AS raw_dim_0",
                "    FROM silver.class_git_pull_requests FINAL",
                "    WHERE tenant_id = ?",
                "      AND toDate(closed_on) >= toDate(?)",
                "      AND toDate(closed_on) <= toDate(?)",
                "),",
                "ranked AS (",
                "    SELECT",
                "        *,",
                "        multiIf((raw_dim_0 = ?), toUInt32(1), (raw_dim_0 = ?), toUInt32(2), toUInt32(0)) AS group_rank",
                "    FROM scoped",
                "),",
                "filtered AS (",
                "    SELECT *",
                "    FROM ranked",
                "),",
                "aggregated AS (",
                "    SELECT",
                "        author_email AS entity_id,",
                "        bucket_start,",
                "        group_rank,",
                "        toFloat64(count()) AS value,",
                "        toUInt8(grouping(bucket_start)) AS is_total",
                "    FROM filtered",
                "    GROUP BY GROUPING SETS (",
                "        (entity_id, bucket_start, group_rank),",
                "        (entity_id, group_rank)",
                "    )",
                ")",
                "SELECT",
                "    entity_id,",
                "    toString(bucket_start) AS bucket_start,",
                "    multiIf(group_rank = 1, toNullable(?), group_rank = 2, toNullable(?), CAST(NULL AS Nullable(String))) AS dim_0_value,",
                "    multiIf(group_rank = 1, toNullable(?), group_rank = 2, CAST(NULL AS Nullable(String)), CAST(NULL AS Nullable(String))) AS dim_0_label,",
                "    value AS value,",
                "    is_total,",
                "    if(group_rank = 0, CAST(NULL AS Nullable(UInt32)), toNullable(group_rank)) AS rank,",
                "    toUInt8(group_rank = 0) AS remainder,",
                "    if(group_rank = 0, toNullable('Other'), CAST(NULL AS Nullable(String))) AS group_label",
                "FROM aggregated",
                "ORDER BY entity_id, group_rank, is_total, bucket_start",
                "LIMIT ?",
            ])
        );
        assert_eq!(
            compiled.params,
            vec![
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                text("example/app"),
                text("example/lib"),
                text("example/app"),
                text("example/lib"),
                text("Example App"),
                QueryParam::UInt(10_001),
            ]
        );
    }

    #[test]
    fn a_cap_that_reports_no_remainder_drops_the_series_outside_its_groups() {
        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            &query(view(&["repository"], Some(cap(false)))),
        );

        assert!(compiled.sql.contains(&lines(&[
            "filtered AS (",
            "    SELECT *",
            "    FROM ranked",
            "    WHERE group_rank > 0",
            "),",
        ])));
    }

    #[test]
    fn a_capped_subject_series_transforms_the_folded_value_in_its_final_stage() {
        let mut metric = metric(direct("prs_merged"));
        metric.transform = Some(percent_of_total());

        let compiled = compile(
            &metric,
            &[labelled_measure("prs_merged")],
            &query(view(&["repository"], Some(cap(true)))),
        );

        assert!(compiled.sql.starts_with("WITH scoped AS ("));
        assert!(compiled.sql.contains(
            "    if((100.0 * (value)) IS NULL, NULL, least(100.0, greatest(0.0, 100.0 * (value)))) AS value,"
        ));
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn a_subject_series_by_a_dimension_the_measure_does_not_declare_is_rejected() {
        for group_limit in [None, Some(cap(true))] {
            assert_eq!(
                compile_err(
                    &metric(direct("prs_merged")),
                    &[labelled_measure("prs_merged")],
                    &query(view(&["team"], group_limit))
                ),
                CompileError::UnknownDimension {
                    measure: "prs_merged".to_owned(),
                    key: "team".to_owned(),
                }
            );
        }
    }

    #[test]
    fn a_cap_over_no_dimension_ranks_nothing_and_is_rejected() {
        assert_eq!(
            compile_err(
                &metric(direct("prs_merged")),
                &[labelled_measure("prs_merged")],
                &query(view(&[], Some(cap(true))))
            ),
            CompileError::EmptySelection {
                selection: "the capped subject-series dimensions".to_owned(),
            }
        );
    }

    #[test]
    fn a_cap_that_names_a_different_number_of_values_than_the_read_groups_by_is_rejected() {
        assert_eq!(
            compile_err(
                &metric(direct("prs_merged")),
                &[labelled_measure("prs_merged")],
                &query(view(&["repository", "source"], Some(cap(true))))
            ),
            CompileError::GroupCapArity {
                rank: 1,
                named: 1,
                requested: 2,
            }
        );
    }
}
