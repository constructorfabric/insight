//! Renders a subject-split read: one value per entity per combination of the
//! dimensions the request names.

use std::fmt::Write;

use crate::domain::definitions::definition::MetricDefinition;
use crate::domain::field_catalog::model::CatalogDataset;

use super::dimensions::{DimensionSource, dimension_select_group};
use super::error::CompileError;
use super::fold::{Fold, ScopedRead, bounded_query};
use super::pool::Pool;
use super::request::MetricQuery;
use super::sql::{CompiledMeasureQuery, ReadScope};

pub(super) fn compile(
    dataset: &CatalogDataset,
    metric: &MetricDefinition,
    fold: &Fold<'_>,
    query: &MetricQuery,
    dimensions: &[String],
    pool: Option<&Pool<'_>>,
) -> Result<CompiledMeasureQuery, CompileError> {
    if dimensions.is_empty() {
        return Err(CompileError::EmptySelection {
            selection: "the subject-split dimensions".to_owned(),
        });
    }

    let (select, group) = dimension_select_group(&DimensionSource::Row(fold.grain), dimensions)?;
    let read = fold.scoped_read(dataset, metric, &ReadScope::of_metric(query), pool)?;
    let inner = subject_split_sql(&read, &select, &group);

    Ok(bounded_query(
        metric.transform.as_ref(),
        read.params,
        query.row_limit,
        inner,
    ))
}

pub(super) fn subject_split_sql(read: &ScopedRead, select: &str, group: &str) -> String {
    let mut sql = read.head.clone();
    sql.push_str("SELECT\n");
    let _ = writeln!(sql, "    {} AS entity_id,", read.entity);
    sql.push_str(select);
    let _ = writeln!(sql, "    {} AS value", read.value);
    let _ = writeln!(sql, "FROM {}", read.scan);
    let _ = writeln!(sql, "WHERE {}", read.predicates.join("\n  AND "));
    let _ = writeln!(sql, "GROUP BY entity_id, {group}");
    if let Some(having) = read.having() {
        let _ = writeln!(sql, "HAVING {having}");
    }
    let _ = writeln!(sql, "ORDER BY entity_id");
    let _ = write!(sql, "LIMIT ?");
    sql
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::domain::compiler::error::CompileError;
    use crate::domain::compiler::fixtures::{
        compile, compile_err, direct, labelled_measure, lines, metric, percent_of_total, query,
        text,
    };
    use crate::domain::compiler::request::{
        DimensionFilter, EntityScope, SubjectSplitView, ViewKind,
    };
    use crate::domain::compiler::sql::QueryParam;

    fn view(dimensions: &[&str]) -> ViewKind {
        ViewKind::SubjectSplit(SubjectSplitView {
            dimensions: dimensions.iter().map(|key| (*key).to_owned()).collect(),
        })
    }

    #[test]
    fn a_subject_split_groups_each_entity_by_every_named_dimension() {
        let measure = labelled_measure("prs_merged");
        let mut request = query(view(&["repository", "source"]));
        request.entity_scope = EntityScope::Identities(vec!["dev@example.com".to_owned()]);
        request.dimension_filters = vec![DimensionFilter {
            key: "source".to_owned(),
            values: vec!["github".to_owned()],
        }];

        let compiled = compile(&metric(direct("prs_merged")), &[measure], &request);

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    author_email AS entity_id,",
                "    coalesce(toString(repo_slug), '__unknown__') AS dim_0_value,",
                "    coalesce(toString(repo_slug), 'Unknown') AS dim_0_label,",
                "    coalesce(toString(data_source), '__unknown__') AS dim_1_value,",
                "    coalesce(toString(data_source_label), 'Unknown') AS dim_1_label,",
                "    toFloat64(count()) AS value",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND author_email IN (?)",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "  AND data_source IN (?)",
                "GROUP BY entity_id, dim_0_value, dim_0_label, dim_1_value, dim_1_label",
                "ORDER BY entity_id",
                "LIMIT ?",
            ])
        );
        assert_eq!(
            compiled.params,
            vec![
                text("acme-tenant"),
                text("dev@example.com"),
                text("2026-01-01"),
                text("2026-01-31"),
                text("github"),
                QueryParam::UInt(10_001),
            ]
        );
    }

    #[test]
    fn a_dimension_the_row_carries_no_value_for_groups_under_the_unknown_sentinel() {
        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            &query(view(&["source"])),
        );

        assert!(
            compiled
                .sql
                .contains("coalesce(toString(data_source), '__unknown__') AS dim_0_value,")
        );
        assert!(
            compiled
                .sql
                .contains("coalesce(toString(data_source_label), 'Unknown') AS dim_0_label,")
        );
    }

    #[test]
    fn a_transform_projects_over_the_grouped_value_and_leaves_the_dimensions_alone() {
        let mut metric = metric(direct("prs_merged"));
        metric.transform = Some(percent_of_total());

        let compiled = compile(
            &metric,
            &[labelled_measure("prs_merged")],
            &query(view(&["repository"])),
        );

        assert!(compiled.sql.starts_with(&lines(&[
            "SELECT",
            "    * EXCEPT (value),",
            "    if((100.0 * (value)) IS NULL, NULL, least(100.0, greatest(0.0, 100.0 * (value)))) AS value",
            "FROM (",
            "SELECT",
            "    author_email AS entity_id,",
            "    coalesce(toString(repo_slug), '__unknown__') AS dim_0_value,",
        ])));
    }

    #[test]
    fn a_subject_split_naming_no_dimension_is_rejected() {
        assert_eq!(
            compile_err(
                &metric(direct("prs_merged")),
                &[labelled_measure("prs_merged")],
                &query(view(&[]))
            ),
            CompileError::EmptySelection {
                selection: "the subject-split dimensions".to_owned(),
            }
        );
    }

    #[test]
    fn a_subject_split_by_a_dimension_the_measure_does_not_declare_is_rejected() {
        assert_eq!(
            compile_err(
                &metric(direct("prs_merged")),
                &[labelled_measure("prs_merged")],
                &query(view(&["team"]))
            ),
            CompileError::UnknownDimension {
                measure: "prs_merged".to_owned(),
                key: "team".to_owned(),
            }
        );
    }
}
