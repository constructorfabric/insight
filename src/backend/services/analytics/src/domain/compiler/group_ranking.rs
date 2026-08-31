//! Renders the group-cap pre-pass: which dimension groups a capped read keeps.
//!
//! Ranking is its own read because the cap it feeds must be the same for every
//! view that shares a cap policy, and because a group's position is decided by
//! the metric's value over the whole window rather than by anything a capped
//! statement could compute mid-flight. A group whose value is unknown is not
//! ranked at all — an unknown value is not a low one.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::domain::definitions::definition::{MeasureDefinition, MetricDefinition};
use crate::domain::field_catalog::model::FieldCatalog;

use super::dimensions::combined_split_dimension_select_group;
use super::error::CompileError;
use super::fold::{Fold, transformed};
use super::request::GroupRankingQuery;
use super::sql::{CompiledMeasureQuery, QueryParam, ReadScope, from_clause};

pub fn compile_group_ranking_query(
    catalog: &FieldCatalog,
    metric: &MetricDefinition,
    measures: &BTreeMap<String, MeasureDefinition>,
    query: &GroupRankingQuery,
) -> Result<CompiledMeasureQuery, CompileError> {
    if query.dimensions.is_empty() {
        return Err(CompileError::EmptySelection {
            selection: "the ranked dimensions".to_owned(),
        });
    }

    let fold = Fold::resolve(metric, measures)?;
    let dataset = fold.dataset(catalog)?;
    let (select, group) = combined_split_dimension_select_group(fold.grain, &query.dimensions)?;
    let read = fold.scoped_read(dataset, metric, &ReadScope::of_ranking(query))?;

    let mut inner = String::from("SELECT\n");
    inner.push_str(&select);
    let _ = writeln!(inner, "    {} AS value", read.value);
    let _ = writeln!(inner, "FROM {}", from_clause(dataset));
    let _ = writeln!(inner, "WHERE {}", read.predicates.join("\n  AND "));
    let _ = write!(inner, "GROUP BY {group}");

    let ranked = transformed(metric.transform.as_ref(), inner);
    let mut params = read.params;
    params.push(QueryParam::UInt(query.count));

    let sql = format!(
        "SELECT *\nFROM (\n{ranked}\n)\nWHERE value IS NOT NULL\nORDER BY value DESC, {group}\nLIMIT ?"
    );

    Ok(CompiledMeasureQuery { sql, params })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use chrono::NaiveDate;

    use crate::domain::compiler::error::CompileError;
    use crate::domain::compiler::fixtures::{
        direct, labelled_measure, lines, measures, metric, percent_of_total, text,
    };
    use crate::domain::compiler::request::EntityScope;
    use crate::domain::compiler::sql::QueryParam;
    use crate::domain::compiler::test_catalog::catalog;
    use crate::domain::definitions::definition::MetricDefinition;

    use super::{GroupRankingQuery, compile_group_ranking_query};

    fn query(dimensions: &[&str]) -> GroupRankingQuery {
        GroupRankingQuery {
            tenant_id: "acme-tenant".to_owned(),
            entity_scope: EntityScope::Tenant,
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
            dimension_filters: Vec::new(),
            dimensions: dimensions.iter().map(|key| (*key).to_owned()).collect(),
            count: 5,
        }
    }

    fn compile(
        metric: &MetricDefinition,
        query: &GroupRankingQuery,
    ) -> super::CompiledMeasureQuery {
        compile_group_ranking_query(
            &catalog(),
            metric,
            &measures(&[labelled_measure("prs_merged")]),
            query,
        )
        .expect("compiles")
    }

    #[test]
    fn ranking_keeps_the_highest_valued_groups_and_never_ranks_an_unknown_value() {
        let compiled = compile(&metric(direct("prs_merged")), &query(&["repository"]));

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT *",
                "FROM (",
                "SELECT",
                "    coalesce(toString(repo_slug), '__unknown__') AS dim_0_value,",
                "    argMax(coalesce(toString(repo_slug), 'Unknown'), tuple(toDate(closed_on), coalesce(toString(repo_slug), 'Unknown'))) AS dim_0_label,",
                "    toFloat64(count()) AS value",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "GROUP BY dim_0_value",
                ")",
                "WHERE value IS NOT NULL",
                "ORDER BY value DESC, dim_0_value",
                "LIMIT ?",
            ])
        );
        assert_eq!(
            compiled.params,
            vec![
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                QueryParam::UInt(5),
            ]
        );
    }

    #[test]
    fn ranking_orders_by_the_transformed_value_the_capped_read_will_report() {
        let mut metric = metric(direct("prs_merged"));
        metric.transform = Some(percent_of_total());

        let compiled = compile(&metric, &query(&["repository", "source"]));

        assert!(compiled.sql.starts_with(&lines(&[
            "SELECT *",
            "FROM (",
            "SELECT",
            "    * EXCEPT (value),",
            "    if((100.0 * (value)) IS NULL, NULL, least(100.0, greatest(0.0, 100.0 * (value)))) AS value",
            "FROM (",
        ])));
        assert!(compiled.sql.ends_with(&lines(&[
            "WHERE value IS NOT NULL",
            "ORDER BY value DESC, dim_0_value, dim_1_value",
            "LIMIT ?",
        ])));
    }

    #[test]
    fn a_ranking_naming_no_dimension_is_rejected() {
        assert_eq!(
            compile_group_ranking_query(
                &catalog(),
                &metric(direct("prs_merged")),
                &measures(&[labelled_measure("prs_merged")]),
                &query(&[]),
            )
            .expect_err("expected a compile error"),
            CompileError::EmptySelection {
                selection: "the ranked dimensions".to_owned(),
            }
        );
    }
}
