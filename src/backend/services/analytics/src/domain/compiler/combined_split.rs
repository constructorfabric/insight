//! Renders a combined-split read: one value per combination of the dimensions the
//! request names, folded over every entity the scope admits.
//!
//! The row carries no entity — it carries how many entities contributed to it
//! — plus the `rank` / `remainder` / `group_label` columns a capped read fills
//! in. An uncapped read reports every group and answers those three with the
//! constants the row decoder expects either way.

use std::fmt::Write;

use crate::domain::definitions::definition::MetricDefinition;
use crate::domain::field_catalog::model::CatalogDataset;

use super::dimensions::combined_split_dimension_select_group;
use super::error::CompileError;
use super::fold::{Fold, ScopedRead, bounded_query, transform_in_place};
use super::group_cap::{
    CAPPED_RANK_COLUMNS, GroupCap, UNCAPPED_RANK_COLUMNS, ranked_scan_ctes, raw_dimension_select,
};
use super::request::{CombinedSplitView, MetricQuery};
use super::sql::{CompiledMeasureQuery, QueryParam, ReadScope, from_clause, read_predicates};

pub(super) fn compile(
    dataset: &CatalogDataset,
    metric: &MetricDefinition,
    fold: &Fold<'_>,
    query: &MetricQuery,
    view: &CombinedSplitView,
) -> Result<CompiledMeasureQuery, CompileError> {
    if view.dimensions.is_empty() {
        return Err(CompileError::EmptySelection {
            selection: "the combined-split dimensions".to_owned(),
        });
    }

    match &view.group_limit {
        None => compile_uncapped(dataset, metric, fold, query, &view.dimensions),
        Some(limit) => {
            let cap = GroupCap::resolve(limit, view.dimensions.len())?;
            compile_capped(dataset, metric, fold, query, &view.dimensions, &cap)
        }
    }
}

fn compile_uncapped(
    dataset: &CatalogDataset,
    metric: &MetricDefinition,
    fold: &Fold<'_>,
    query: &MetricQuery,
    dimensions: &[String],
) -> Result<CompiledMeasureQuery, CompileError> {
    let (select, group) = combined_split_dimension_select_group(fold.grain, dimensions)?;
    let read = fold.scoped_read(dataset, metric, &ReadScope::of_metric(query))?;
    let inner = uncapped_sql(dataset, fold, &read, &select, &group);

    Ok(bounded_query(
        metric.transform.as_ref(),
        read.params,
        query.row_limit,
        inner,
    ))
}

fn uncapped_sql(
    dataset: &CatalogDataset,
    fold: &Fold<'_>,
    read: &ScopedRead,
    select: &str,
    group: &str,
) -> String {
    let mut sql = String::from("SELECT\n");
    sql.push_str(select);
    let _ = writeln!(sql, "    {} AS value,", read.value);
    let _ = writeln!(
        sql,
        "    uniqExact({}) AS contributing_entity_count,",
        fold.grain.entity
    );
    sql.push_str(UNCAPPED_RANK_COLUMNS);
    let _ = writeln!(sql, "FROM {}", from_clause(dataset));
    let _ = writeln!(sql, "WHERE {}", read.predicates.join("\n  AND "));
    let _ = writeln!(sql, "GROUP BY {group}");
    let _ = writeln!(sql, "ORDER BY {group}");
    let _ = write!(sql, "LIMIT ?");
    sql
}

/// A capped read ranks each scanned row before it folds anything, so the scan
/// is written before the fold and its values bind first. The transform is
/// projected in the final stage rather than wrapped around it: the cap already
/// owns that stage.
fn compile_capped(
    dataset: &CatalogDataset,
    metric: &MetricDefinition,
    fold: &Fold<'_>,
    query: &MetricQuery,
    dimensions: &[String],
    cap: &GroupCap<'_>,
) -> Result<CompiledMeasureQuery, CompileError> {
    let raw_dimensions = raw_dimension_select(fold.grain, dimensions)?;

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
        &raw_dimensions,
        &predicates,
        &rank,
        cap.remainder_predicate(),
    );
    let _ = writeln!(sql, "aggregated AS (");
    let _ = writeln!(sql, "    SELECT");
    let _ = writeln!(sql, "        group_rank,");
    let _ = writeln!(sql, "        {value} AS value,");
    let _ = writeln!(
        sql,
        "        uniqExact({}) AS contributing_entity_count",
        fold.grain.entity
    );
    let _ = writeln!(sql, "    FROM filtered");
    let _ = writeln!(sql, "    GROUP BY group_rank");
    let _ = writeln!(sql, ")");
    let _ = writeln!(sql, "SELECT");
    let _ = writeln!(sql, "    group_rank,");
    sql.push_str(&dimension_select);
    let _ = writeln!(
        sql,
        "    {} AS value,",
        transform_in_place(metric.transform.as_ref(), "value")
    );
    let _ = writeln!(sql, "    contributing_entity_count,");
    sql.push_str(CAPPED_RANK_COLUMNS);
    let _ = writeln!(sql, "FROM aggregated");
    let _ = writeln!(sql, "ORDER BY group_rank = 0, group_rank");
    let _ = write!(sql, "LIMIT ?");

    Ok(CompiledMeasureQuery { sql, params })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::domain::compiler::error::CompileError;
    use crate::domain::compiler::fixtures::{
        compile, compile_err, direct, labelled_measure, lines, measure, metric, percent_of_total,
        query, ratio, text,
    };
    use crate::domain::compiler::request::{
        CombinedSplitView, GroupLimit, RankedDimension, RankedGroup, ViewKind,
    };
    use crate::domain::compiler::sql::QueryParam;

    fn view(dimensions: &[&str], group_limit: Option<GroupLimit>) -> ViewKind {
        ViewKind::CombinedSplit(CombinedSplitView {
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
    fn an_uncapped_combined_split_reports_every_group_and_who_contributed_to_it() {
        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            &query(view(&["repository"], None)),
        );

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    coalesce(toString(repo_slug), '__unknown__') AS dim_0_value,",
                "    argMax(coalesce(toString(repo_slug), 'Unknown'), tuple(toDate(closed_on), coalesce(toString(repo_slug), 'Unknown'))) AS dim_0_label,",
                "    toFloat64(count()) AS value,",
                "    uniqExact(author_email) AS contributing_entity_count,",
                "    CAST(NULL AS Nullable(UInt32)) AS rank,",
                "    toUInt8(0) AS remainder,",
                "    CAST(NULL AS Nullable(String)) AS group_label",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "GROUP BY dim_0_value",
                "ORDER BY dim_0_value",
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
    fn a_capped_combined_split_ranks_each_scanned_row_before_it_folds_anything() {
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
                "        group_rank,",
                "        toFloat64(count()) AS value,",
                "        uniqExact(author_email) AS contributing_entity_count",
                "    FROM filtered",
                "    GROUP BY group_rank",
                ")",
                "SELECT",
                "    group_rank,",
                "    multiIf(group_rank = 1, toNullable(?), group_rank = 2, toNullable(?), CAST(NULL AS Nullable(String))) AS dim_0_value,",
                "    multiIf(group_rank = 1, toNullable(?), group_rank = 2, CAST(NULL AS Nullable(String)), CAST(NULL AS Nullable(String))) AS dim_0_label,",
                "    value AS value,",
                "    contributing_entity_count,",
                "    if(group_rank = 0, CAST(NULL AS Nullable(UInt32)), toNullable(group_rank)) AS rank,",
                "    toUInt8(group_rank = 0) AS remainder,",
                "    if(group_rank = 0, toNullable('Other'), CAST(NULL AS Nullable(String))) AS group_label",
                "FROM aggregated",
                "ORDER BY group_rank = 0, group_rank",
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
    fn a_cap_that_reports_no_remainder_drops_the_rows_outside_its_groups() {
        let kept = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            &query(view(&["repository"], Some(cap(false)))),
        );

        assert!(kept.sql.contains(&lines(&[
            "filtered AS (",
            "    SELECT *",
            "    FROM ranked",
            "    WHERE group_rank > 0",
            "),",
        ])));
    }

    #[test]
    fn a_cap_that_kept_no_group_ranks_every_row_into_the_remainder() {
        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            &query(view(
                &["repository"],
                Some(GroupLimit {
                    groups: Vec::new(),
                    include_remainder: true,
                }),
            )),
        );

        assert!(compiled.sql.contains("        toUInt32(0) AS group_rank"));
        assert!(
            compiled
                .sql
                .contains("    CAST(NULL AS Nullable(String)) AS dim_0_value,")
        );
        assert!(
            compiled
                .sql
                .contains("    CAST(NULL AS Nullable(String)) AS dim_0_label,")
        );
    }

    #[test]
    fn a_capped_combined_split_transforms_the_folded_value_in_its_final_stage() {
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
    }

    #[test]
    fn a_combined_split_naming_no_dimension_is_rejected() {
        assert_eq!(
            compile_err(
                &metric(direct("prs_merged")),
                &[labelled_measure("prs_merged")],
                &query(view(&[], None))
            ),
            CompileError::EmptySelection {
                selection: "the combined-split dimensions".to_owned(),
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

    #[test]
    fn a_ratio_combined_split_binds_its_fold_after_the_scan_it_ranks() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let created = measure("prs_created", None);

        let compiled = compile(
            &metric(ratio("prs_merged", "prs_created")),
            &[merged, created],
            &query(view(&["repository"], Some(cap(true)))),
        );

        assert!(compiled.sql.contains(
            "        toFloat64(countIfOrNull(state = ?) / nullIf(countIf(1), 0)) AS value,"
        ));
        assert_eq!(compiled.params[0], text("acme-tenant"));
        assert_eq!(compiled.params[3], text("example/app"));
        assert_eq!(compiled.params[5], text("merged"));
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }
}
