//! Renders a bins read: how many of an entity's own observations fall in
//! each bin of that entity's range.
//!
//! INVARIANT: binning is fixed-width arithmetic over each entity's exact
//! minimum and maximum, so identical rows always bin identically.

use std::fmt::Write;

use crate::domain::definitions::definition::MetricDefinition;
use crate::domain::field_catalog::model::CatalogDataset;

use super::error::CompileError;
use super::fold::{Fold, transform_in_place};
use super::pool::{Pool, first_cte, joined_entity, scan_clause};
use super::request::MetricQuery;
use super::sql::{CompiledMeasureQuery, QueryParam, ReadScope, read_predicates};

/// How many bins an entity's range is cut into.
const BINS: u32 = 10;
const LAST_BIN: u32 = BINS - 1;

pub(super) fn compile(
    dataset: &CatalogDataset,
    metric: &MetricDefinition,
    fold: &Fold<'_>,
    query: &MetricQuery,
    pool: Option<&Pool<'_>>,
) -> Result<CompiledMeasureQuery, CompileError> {
    let row_value = fold.row_value_expr(metric)?;
    let binned = transform_in_place(metric.transform.as_ref(), row_value);

    let mut params = Vec::new();
    let head = first_cte(pool, &mut params)?;
    let mut predicates = read_predicates(
        dataset,
        fold.grain,
        fold.where_filter,
        &ReadScope::of_metric(query),
        &mut params,
    )?;
    predicates.push(format!("{row_value} IS NOT NULL"));
    params.push(QueryParam::UInt(query.row_limit));

    let mut sql = head;
    sql.push_str("raw_events AS (\n    SELECT\n");
    let _ = writeln!(
        sql,
        "        {} AS entity_id,",
        joined_entity(pool, fold.grain)
    );
    let _ = writeln!(sql, "        assumeNotNull({binned}) AS event_value");
    let _ = writeln!(
        sql,
        "    FROM {}",
        scan_clause(dataset, pool, fold.grain, "    ")
    );
    let _ = writeln!(sql, "    WHERE {}", predicates.join("\n      AND "));
    let _ = writeln!(sql, "),");
    let _ = writeln!(sql, "events AS (");
    let _ = writeln!(sql, "    SELECT");
    let _ = writeln!(sql, "        entity_id,");
    let _ = writeln!(sql, "        event_value,");
    let _ = writeln!(
        sql,
        "        min(event_value) OVER (PARTITION BY entity_id) AS entity_lo,"
    );
    let _ = writeln!(
        sql,
        "        max(event_value) OVER (PARTITION BY entity_id) AS entity_hi"
    );
    let _ = writeln!(sql, "    FROM raw_events");
    let _ = writeln!(sql, ")");
    let _ = writeln!(sql, "SELECT");
    let _ = writeln!(
        sql,
        "    toString(assumeNotNull(events.entity_id)) AS entity_id,"
    );
    // INVARIANT: a range collapsing to a point maps every observation to bin 0,
    // and `least` closes the last bin on the maximum rather than opening one more.
    let _ = writeln!(sql, "    if(");
    let _ = writeln!(sql, "        events.entity_hi = events.entity_lo,");
    let _ = writeln!(sql, "        0,");
    let _ = writeln!(sql, "        toUInt32(least({LAST_BIN}, toInt64(floor(");
    let _ = writeln!(
        sql,
        "            (events.event_value - events.entity_lo) * {BINS} / (events.entity_hi - events.entity_lo)"
    );
    let _ = writeln!(sql, "        ))))");
    let _ = writeln!(sql, "    ) AS bin_idx,");
    let _ = writeln!(sql, "    any(events.entity_lo) AS entity_lo,");
    let _ = writeln!(sql, "    any(events.entity_hi) AS entity_hi,");
    let _ = writeln!(sql, "    toUInt64(count()) AS bin_count");
    let _ = writeln!(sql, "FROM events");
    let _ = writeln!(sql, "GROUP BY entity_id, bin_idx");
    let _ = writeln!(sql, "ORDER BY entity_id, bin_idx");
    let _ = write!(sql, "LIMIT ?");

    Ok(CompiledMeasureQuery { sql, params })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::domain::compiler::error::CompileError;
    use crate::domain::compiler::fixtures::{
        compile, compile_err, direct, lines, measure, metric, percent_of_total, percentile, query,
        sized_measure, text,
    };
    use crate::domain::compiler::request::ViewKind;
    use crate::domain::compiler::sql::QueryParam;

    #[test]
    fn a_bins_read_bins_each_entitys_own_values_over_its_own_range() {
        let compiled = compile(
            &metric(percentile("pr_size", 0.5)),
            &[sized_measure("pr_size")],
            &query(ViewKind::Bins),
        );

        assert_eq!(
            compiled.sql,
            lines(&[
                "WITH raw_events AS (",
                "    SELECT",
                "        author_email AS entity_id,",
                "        assumeNotNull(lines_added) AS event_value",
                "    FROM silver.class_git_pull_requests FINAL",
                "    WHERE tenant_id = ?",
                "      AND toDate(closed_on) >= toDate(?)",
                "      AND toDate(closed_on) <= toDate(?)",
                "      AND lines_added IS NOT NULL",
                "),",
                "events AS (",
                "    SELECT",
                "        entity_id,",
                "        event_value,",
                "        min(event_value) OVER (PARTITION BY entity_id) AS entity_lo,",
                "        max(event_value) OVER (PARTITION BY entity_id) AS entity_hi",
                "    FROM raw_events",
                ")",
                "SELECT",
                "    toString(assumeNotNull(events.entity_id)) AS entity_id,",
                "    if(",
                "        events.entity_hi = events.entity_lo,",
                "        0,",
                "        toUInt32(least(9, toInt64(floor(",
                "            (events.event_value - events.entity_lo) * 10 / (events.entity_hi - events.entity_lo)",
                "        ))))",
                "    ) AS bin_idx,",
                "    any(events.entity_lo) AS entity_lo,",
                "    any(events.entity_hi) AS entity_hi,",
                "    toUInt64(count()) AS bin_count",
                "FROM events",
                "GROUP BY entity_id, bin_idx",
                "ORDER BY entity_id, bin_idx",
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
    fn the_binned_value_is_the_transformed_one_and_the_null_check_the_raw_one() {
        let mut metric = metric(percentile("pr_size", 0.5));
        metric.transform = Some(percent_of_total());

        let compiled = compile(&metric, &[sized_measure("pr_size")], &query(ViewKind::Bins));

        assert!(compiled.sql.contains(
            "        assumeNotNull(if((100.0 * (lines_added)) IS NULL, NULL, least(100.0, greatest(0.0, 100.0 * (lines_added))))) AS event_value"
        ));
        assert!(compiled.sql.contains("      AND lines_added IS NOT NULL"));
    }

    #[test]
    fn a_metric_that_folds_no_per_row_value_has_no_bins_read() {
        assert_eq!(
            compile_err(
                &metric(direct("prs_merged")),
                &[measure("prs_merged", None)],
                &query(ViewKind::Bins)
            ),
            CompileError::UnsupportedView {
                metric: "git.merge_rate".to_owned(),
                view: "bins",
                reason: "it needs a percentile or stddev computation, the only ones taken over the measure's own per-row values",
            }
        );
    }

    #[test]
    fn a_percentile_over_a_measure_that_folds_no_value_is_rejected() {
        assert_eq!(
            compile_err(
                &metric(percentile("prs_merged", 0.5)),
                &[measure("prs_merged", None)],
                &query(ViewKind::Bins)
            ),
            CompileError::DistributionWithoutValue {
                metric: "git.merge_rate".to_owned(),
                measure: "prs_merged".to_owned(),
            }
        );
    }
}
