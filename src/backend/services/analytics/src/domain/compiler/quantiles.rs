//! Renders a quantile read: where an entity's own observations sit, at the
//! positions the caller named.
//!
//! INVARIANT: the positions are taken over the per-row values a bins read cuts
//! and a percentile metric ranks, by the same exact family.

use std::fmt::Write;

use crate::domain::definitions::definition::MetricDefinition;
use crate::domain::field_catalog::model::CatalogDataset;

use super::error::CompileError;
use super::fold::{Fold, transform_in_place};
use super::pool::{Pool, joined_entity, only_cte, scan_clause};
use super::request::{MetricQuery, QuantilesView};
use super::sql::{CompiledMeasureQuery, QueryParam, ReadScope, from_clause, read_predicates};

pub(super) fn compile(
    dataset: &CatalogDataset,
    metric: &MetricDefinition,
    fold: &Fold<'_>,
    query: &MetricQuery,
    view: &QuantilesView,
    pool: Option<&Pool<'_>>,
) -> Result<CompiledMeasureQuery, CompileError> {
    if view.quantiles.is_empty() {
        return Err(CompileError::EmptySelection {
            selection: "the quantiles".to_owned(),
        });
    }

    let row_value = fold.row_value_expr(metric, "quantiles")?;
    let ranked = transform_in_place(metric.transform.as_ref(), row_value);

    let mut params = Vec::new();
    let head = only_cte(pool, &mut params)?;
    let mut predicates = read_predicates(
        dataset,
        fold.grain,
        fold.where_filter,
        &ReadScope::of_metric(query),
        &mut params,
    )?;
    predicates.push(format!("{row_value} IS NOT NULL"));
    params.push(QueryParam::UInt(query.row_limit));

    // SAFETY: each position is a parsed `f64` the request validated into
    // `(0, 1)`; a parametric aggregate takes constants, so it is written rather
    // than bound, and no spelling of a float can carry SQL.
    let positions = view
        .quantiles
        .iter()
        .map(|quantile| format!("{quantile:?}"))
        .collect::<Vec<_>>()
        .join(", ");

    let mut sql = head;
    sql.push_str("SELECT\n");
    let _ = writeln!(
        sql,
        "    {} AS entity_id,",
        joined_entity(pool, &fold.grain.entity)
    );
    let _ = writeln!(
        sql,
        "    quantilesExact({positions})(assumeNotNull({ranked})) AS quantile_values"
    );
    let _ = writeln!(
        sql,
        "FROM {}",
        scan_clause(from_clause(dataset), pool, &fold.grain.entity, "")
    );
    let _ = writeln!(sql, "WHERE {}", predicates.join("\n  AND "));
    let _ = writeln!(sql, "GROUP BY entity_id");
    let _ = writeln!(sql, "ORDER BY entity_id");
    let _ = write!(sql, "LIMIT ?");

    Ok(CompiledMeasureQuery { sql, params })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::domain::compiler::error::CompileError;
    use crate::domain::compiler::fixtures::{
        compile, compile_err, direct, lines, measure, metric, people, people_params,
        percent_of_total, percentile, quantiles, query, sized_measure, text,
    };
    use crate::domain::compiler::sql::QueryParam;

    #[test]
    fn a_quantile_read_takes_every_named_position_over_each_entitys_own_values() {
        let compiled = compile(
            &metric(percentile("pr_size", 0.5)),
            &[sized_measure("pr_size")],
            &query(quantiles(&[0.5, 0.9])),
        );

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    author_email AS entity_id,",
                "    quantilesExact(0.5, 0.9)(assumeNotNull(lines_added)) AS quantile_values",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "  AND lines_added IS NOT NULL",
                "GROUP BY entity_id",
                "ORDER BY entity_id",
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
    fn a_people_scoped_read_reaches_its_entities_through_the_pool_and_keys_rows_by_the_person() {
        let mut request = query(quantiles(&[0.5]));
        request.entity_scope = people();

        let compiled = compile(
            &metric(percentile("pr_size", 0.5)),
            &[sized_measure("pr_size")],
            &request,
        );

        assert!(compiled.sql.contains("    pool.person_ref AS entity_id,"));
        assert!(
            compiled
                .sql
                .contains("INNER JOIN pool ON pool.identity = author_email")
        );
        assert_eq!(compiled.params[..6], people_params()[..]);
    }

    #[test]
    fn the_ranked_value_is_the_transformed_one_and_the_null_check_the_raw_one() {
        let mut metric = metric(percentile("pr_size", 0.5));
        metric.transform = Some(percent_of_total());

        let compiled = compile(
            &metric,
            &[sized_measure("pr_size")],
            &query(quantiles(&[0.5])),
        );

        assert!(compiled.sql.contains(
            "    quantilesExact(0.5)(assumeNotNull(if((100.0 * (lines_added)) IS NULL, NULL, least(100.0, greatest(0.0, 100.0 * (lines_added)))))) AS quantile_values"
        ));
        assert!(compiled.sql.contains("  AND lines_added IS NOT NULL"));
    }

    #[test]
    fn a_metric_that_folds_no_per_row_value_has_no_quantiles() {
        assert_eq!(
            compile_err(
                &metric(direct("prs_merged")),
                &[measure("prs_merged", None)],
                &query(quantiles(&[0.5]))
            ),
            CompileError::UnsupportedView {
                metric: "git.merge_rate".to_owned(),
                view: "quantiles",
                reason: "it needs a percentile or stddev computation, the only ones taken over the measure's own per-row values",
            }
        );
    }

    #[test]
    fn a_read_naming_no_position_reports_nothing_and_is_rejected() {
        assert_eq!(
            compile_err(
                &metric(percentile("pr_size", 0.5)),
                &[sized_measure("pr_size")],
                &query(quantiles(&[]))
            ),
            CompileError::EmptySelection {
                selection: "the quantiles".to_owned(),
            }
        );
    }
}
