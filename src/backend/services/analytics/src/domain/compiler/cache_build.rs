//! Renders one measure's cache build: the `INSERT ... SELECT` that materializes
//! its rows at the finest grain every read over it can re-aggregate from.
//!
//! SAFETY: catalog-validated field names and expressions are written into the
//! statement; keys, versions, dates and filter values are all bound.

use std::fmt::Write;

use chrono::NaiveDate;

use crate::domain::definitions::definition::{
    Aggregation, Computation, MeasureDefinition, MetricDefinition,
};
use crate::domain::field_catalog::model::{CatalogDataset, FieldRole};

use super::dimensions::{dimension_label_expr, dimension_value_expr};
use super::error::CompileError;
use super::request::Bucket;
use super::sql::{
    CompiledMeasureQuery, QueryParam, TimeBucket, aggregate_expr, from_clause, render_filter,
    subject_operand, value_operand,
};

pub const CACHE_RELATION: &str = "insight.semantic_measure_cache";
pub const STAGING_RELATION: &str = "insight.semantic_measure_cache_staging";

/// INVARIANT: the served relation and its staging twin share this column order;
/// `INSERT ... SELECT` matches by position.
const CACHE_COLUMNS: &str = "(tenant_id, measure_key, definition_version, kind, metric_date, \
     entity, dimensions, value, subject, built_at)";

const DIMENSIONS_TYPE: &str = "Array(Tuple(key String, value String, label Nullable(String)))";

/// The row shape a measure's cached work takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheRowKind {
    /// One row per tenant, entity, day and dimension tuple.
    Aggregate,
    /// One row per source event, so a distribution over the values stays exact.
    Event,
    /// One row per counted subject, so a distinct count over any span of days
    /// counts each subject once.
    Subject,
}

impl CacheRowKind {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Aggregate => "aggregate",
            Self::Event => "event",
            Self::Subject => "subject",
        }
    }

    /// The shape a stored spelling names; `None` for one this release does not
    /// write, which no read may guess a fold for.
    pub fn from_db(stored: &str) -> Option<Self> {
        match stored {
            "aggregate" => Some(Self::Aggregate),
            "event" => Some(Self::Event),
            "subject" => Some(Self::Subject),
            _ => None,
        }
    }
}

/// What a build is asked to materialize: one measure at one version over one
/// span of days.
#[derive(Debug, Clone, Copy)]
pub struct CacheBuild<'a> {
    pub measure: &'a MeasureDefinition,
    pub definition_version: u32,
    pub kind: CacheRowKind,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

/// The finest reusable form: rows are kept whenever folding them away would
/// cost an answer, and folded otherwise.
pub fn row_kind(measure: &MeasureDefinition, metrics: &[MetricDefinition]) -> CacheRowKind {
    if read_as_distribution(measure, metrics) {
        return CacheRowKind::Event;
    }

    match measure.aggregation {
        // INVARIANT: an average carries no count to re-weight with, so a
        // day-folded average cannot re-fold across a wider window.
        Aggregation::Avg => CacheRowKind::Event,
        Aggregation::CountDistinct => CacheRowKind::Subject,
        Aggregation::Count | Aggregation::Sum | Aggregation::Min | Aggregation::Max => {
            CacheRowKind::Aggregate
        }
    }
}

/// A distribution is taken over per-row values, so a measure any metric reads
/// that way keeps its rows whatever its own fold is.
fn read_as_distribution(measure: &MeasureDefinition, metrics: &[MetricDefinition]) -> bool {
    metrics.iter().any(|metric| match &metric.computation {
        Computation::Percentile { measure: read, .. } | Computation::Stddev { measure: read } => {
            *read == measure.key
        }
        Computation::Direct { .. } | Computation::Ratio { .. } | Computation::Derived { .. } => {
            false
        }
    })
}

/// The build always lands in staging; the refresher swaps it into the served
/// relation one partition at a time.
pub fn compile_cache_build(
    dataset: &CatalogDataset,
    build: &CacheBuild<'_>,
) -> Result<CompiledMeasureQuery, CompileError> {
    let measure = build.measure;
    let tenant_field = dataset
        .fields_with_role(FieldRole::Tenant)
        .next()
        .ok_or_else(|| CompileError::NoTenantField {
            dataset: dataset.key.clone(),
        })?;

    let mut params = vec![
        QueryParam::Text(measure.key.clone()),
        QueryParam::UInt(u64::from(build.definition_version)),
    ];
    let dimensions = dimensions_expr(measure, &mut params);
    let value = value_expr(build)?;
    let subject = subject_expr(build)?;

    let metric_date = TimeBucket::over_event_time(dataset, &measure.event_time, Bucket::Day);
    // SAFETY: the tenant's `assumeNotNull` is sound under the guard below, and
    // a row naming no tenant is reachable by no request.
    let mut predicates = vec![
        format!("{} IS NOT NULL", tenant_field.name),
        format!("{} >= toDate(?)", metric_date.expr()),
        format!("{} <= toDate(?)", metric_date.expr()),
    ];
    params.push(QueryParam::Text(build.from.to_string()));
    params.push(QueryParam::Text(build.to.to_string()));
    metric_date.exclude_timeless(&mut predicates);
    if let Some(filter) = &measure.filter {
        predicates.push(render_filter(measure, filter, &mut params)?);
    }

    let mut sql = format!("INSERT INTO {STAGING_RELATION} {CACHE_COLUMNS}\n");
    sql.push_str("SELECT\n");
    let _ = writeln!(
        sql,
        "    assumeNotNull({}) AS cache_tenant,",
        tenant_field.name
    );
    let _ = writeln!(sql, "    ? AS cache_measure_key,");
    let _ = writeln!(sql, "    ? AS cache_definition_version,");
    let _ = writeln!(sql, "    '{}' AS cache_kind,", build.kind.as_db());
    let _ = writeln!(sql, "    {} AS cache_metric_date,", metric_date.expr());
    let _ = writeln!(sql, "    {} AS cache_entity,", measure.entity);
    let _ = writeln!(sql, "    {dimensions} AS cache_dimensions,");
    let _ = writeln!(sql, "    {value} AS cache_value,");
    let _ = writeln!(sql, "    {subject} AS cache_subject,");
    let _ = writeln!(sql, "    now64(3) AS cache_built_at");
    let _ = writeln!(sql, "FROM {}", from_clause(dataset));
    let _ = write!(sql, "WHERE {}", predicates.join("\n  AND "));
    if let Some(group_by) = group_columns(build.kind) {
        let _ = write!(sql, "\nGROUP BY {group_by}");
    }

    Ok(CompiledMeasureQuery { sql, params })
}

/// A pre-folded row is one per group; an event row is the source row itself.
fn group_columns(kind: CacheRowKind) -> Option<&'static str> {
    match kind {
        CacheRowKind::Aggregate => {
            Some("cache_tenant, cache_metric_date, cache_entity, cache_dimensions")
        }
        CacheRowKind::Subject => {
            Some("cache_tenant, cache_metric_date, cache_entity, cache_dimensions, cache_subject")
        }
        CacheRowKind::Event => None,
    }
}

fn value_expr(build: &CacheBuild<'_>) -> Result<String, CompileError> {
    match build.kind {
        CacheRowKind::Aggregate => Ok(format!("toFloat64({})", aggregate_expr(build.measure)?)),
        CacheRowKind::Event => Ok(format!("toFloat64({})", value_operand(build.measure)?)),
        // A subject row is a presence, and the count of them is the answer.
        CacheRowKind::Subject => Ok("toFloat64(1)".to_owned()),
    }
}

fn subject_expr(build: &CacheBuild<'_>) -> Result<String, CompileError> {
    match build.kind {
        CacheRowKind::Aggregate | CacheRowKind::Event => {
            Ok("CAST(NULL AS Nullable(String))".to_owned())
        }
        CacheRowKind::Subject => Ok(format!("toString({})", subject_operand(build.measure)?)),
    }
}

/// Every declared dimension travels with the row, under the same value and
/// label expressions a live read groups by, so a cached answer and a live one
/// agree on what a group is.
fn dimensions_expr(measure: &MeasureDefinition, params: &mut Vec<QueryParam>) -> String {
    let mut tuples = Vec::with_capacity(measure.dimensions.len());
    for binding in &measure.dimensions {
        params.push(QueryParam::Text(binding.key.clone()));
        tuples.push(format!(
            "tuple(?, {}, {})",
            dimension_value_expr(binding),
            dimension_label_expr(binding)
        ));
    }

    format!("CAST([{}], '{DIMENSIONS_TYPE}')", tuples.join(", "))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::domain::compiler::test_catalog::dataset;
    use crate::domain::definitions::definition::{
        DimensionBinding, Direction, Format, MetricDefinition,
    };
    use crate::domain::definitions::filter::{
        FilterLeaf, FilterOp, FilterTree, FilterValue, Scalar,
    };
    use crate::domain::field_catalog::model::EntityType;

    fn measure() -> MeasureDefinition {
        MeasureDefinition {
            key: "prs_merged".to_owned(),
            dataset: "git_pull_requests".to_owned(),
            description: None,
            filter: serde_yaml::from_str("{ field: state, op: eq, value: merged }").ok(),
            aggregation: Aggregation::Count,
            value_expr: None,
            subject_expr: None,
            event_time: "closed_on".to_owned(),
            entity: "author_email".to_owned(),
            dimensions: vec![
                DimensionBinding {
                    key: "repository".to_owned(),
                    value_field: "repo_slug".to_owned(),
                    label_field: None,
                },
                DimensionBinding {
                    key: "source".to_owned(),
                    value_field: "data_source".to_owned(),
                    label_field: Some("data_source_label".to_owned()),
                },
            ],
        }
    }

    fn build(measure: &MeasureDefinition, kind: CacheRowKind) -> CacheBuild<'_> {
        CacheBuild {
            measure,
            definition_version: 7,
            kind,
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 2, 4).expect("valid date"),
        }
    }

    fn compile(measure: &MeasureDefinition, kind: CacheRowKind) -> CompiledMeasureQuery {
        compile_cache_build(&dataset(&measure.dataset), &build(measure, kind)).expect("compiles")
    }

    fn text(value: &str) -> QueryParam {
        QueryParam::Text(value.to_owned())
    }

    fn lines(expected: &[&str]) -> String {
        expected.join("\n")
    }

    fn metric(key: &str, computation: Computation) -> MetricDefinition {
        MetricDefinition {
            key: key.to_owned(),
            computation,
            transform: None,
            format: Format::Decimal,
            direction: Direction::HigherIsBetter,
            entity_type: EntityType::Person,
            cohort_key: None,
            label: None,
            description: None,
        }
    }

    #[test]
    fn an_aggregate_build_folds_one_row_per_entity_day_and_dimension_tuple() {
        let compiled = compile(&measure(), CacheRowKind::Aggregate);

        assert_eq!(
            compiled.sql,
            lines(&[
                "INSERT INTO insight.semantic_measure_cache_staging (tenant_id, measure_key, definition_version, kind, metric_date, entity, dimensions, value, subject, built_at)",
                "SELECT",
                "    assumeNotNull(tenant_id) AS cache_tenant,",
                "    ? AS cache_measure_key,",
                "    ? AS cache_definition_version,",
                "    'aggregate' AS cache_kind,",
                "    toDate(assumeNotNull(closed_on)) AS cache_metric_date,",
                "    author_email AS cache_entity,",
                "    CAST([tuple(?, coalesce(toString(repo_slug), '__unknown__'), coalesce(toString(repo_slug), 'Unknown')), tuple(?, coalesce(toString(data_source), '__unknown__'), coalesce(toString(data_source_label), 'Unknown'))], 'Array(Tuple(key String, value String, label Nullable(String)))') AS cache_dimensions,",
                "    toFloat64(count()) AS cache_value,",
                "    CAST(NULL AS Nullable(String)) AS cache_subject,",
                "    now64(3) AS cache_built_at",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id IS NOT NULL",
                "  AND toDate(assumeNotNull(closed_on)) >= toDate(?)",
                "  AND toDate(assumeNotNull(closed_on)) <= toDate(?)",
                "  AND isNotNull(closed_on)",
                "  AND state = ?",
                "GROUP BY cache_tenant, cache_metric_date, cache_entity, cache_dimensions",
            ])
        );
        assert_eq!(
            compiled.params,
            vec![
                text("prs_merged"),
                QueryParam::UInt(7),
                text("repository"),
                text("source"),
                text("2026-01-01"),
                text("2026-02-04"),
                text("merged"),
            ]
        );
    }

    #[test]
    fn an_event_build_keeps_one_row_per_source_row() {
        let mut measure = measure();
        measure.aggregation = Aggregation::Avg;
        measure.value_expr = Some("lines_added".to_owned());

        let compiled = compile(&measure, CacheRowKind::Event);

        assert!(compiled.sql.contains("    'event' AS cache_kind,"));
        assert!(
            compiled
                .sql
                .contains("    toFloat64(lines_added) AS cache_value,")
        );
        assert!(
            !compiled.sql.contains("GROUP BY"),
            "a per-event row is not folded: {}",
            compiled.sql
        );
    }

    #[test]
    fn a_subject_build_keeps_one_row_per_counted_subject() {
        let mut measure = measure();
        measure.aggregation = Aggregation::CountDistinct;
        measure.subject_expr = Some("pull_request_id".to_owned());

        let compiled = compile(&measure, CacheRowKind::Subject);

        assert!(compiled.sql.contains("    'subject' AS cache_kind,"));
        assert!(compiled.sql.contains("    toFloat64(1) AS cache_value,"));
        assert!(
            compiled
                .sql
                .contains("    toString(pull_request_id) AS cache_subject,")
        );
        assert!(compiled.sql.contains(
            "GROUP BY cache_tenant, cache_metric_date, cache_entity, cache_dimensions, cache_subject"
        ));
    }

    #[test]
    fn a_collapsing_dataset_is_read_final_and_a_direct_one_is_not() {
        let mut measure = measure();
        measure.filter = None;
        assert!(
            compile(&measure, CacheRowKind::Aggregate)
                .sql
                .contains("FROM silver.class_git_pull_requests FINAL\n")
        );

        measure.dataset = "git_commits".to_owned();
        measure.event_time = "committed_on".to_owned();
        measure.dimensions = vec![DimensionBinding {
            key: "repository".to_owned(),
            value_field: "repo_slug".to_owned(),
            label_field: None,
        }];

        let compiled = compile(&measure, CacheRowKind::Aggregate);

        assert!(compiled.sql.contains("FROM silver.class_git_commits\n"));
        assert!(!compiled.sql.contains("FINAL"));
    }

    #[test]
    fn an_event_time_no_row_leaves_empty_is_dated_without_a_timeless_guard() {
        let mut measure = measure();
        measure.filter = None;
        measure.dataset = "git_commits".to_owned();
        measure.event_time = "committed_on".to_owned();
        measure.dimensions = vec![DimensionBinding {
            key: "repository".to_owned(),
            value_field: "repo_slug".to_owned(),
            label_field: None,
        }];

        let compiled = compile(&measure, CacheRowKind::Aggregate);

        assert!(
            compiled
                .sql
                .contains("    toDate(committed_on) AS cache_metric_date,"),
            "{}",
            compiled.sql
        );
        assert!(
            !compiled.sql.contains("isNotNull"),
            "no row is guarded away: {}",
            compiled.sql
        );
    }

    #[test]
    fn a_build_scopes_every_tenant_and_narrows_none() {
        let compiled = compile(&measure(), CacheRowKind::Aggregate);

        assert!(
            !compiled.sql.contains("tenant_id = ?"),
            "the cache is all-tenant: {}",
            compiled.sql
        );
        assert!(
            compiled
                .sql
                .contains("assumeNotNull(tenant_id) AS cache_tenant")
        );
    }

    #[test]
    fn no_authored_value_reaches_the_statement_text() {
        let injection = "'; DROP TABLE x; --";
        let mut measure = measure();
        measure.key = injection.to_owned();
        measure.dimensions[0].key = injection.to_owned();
        measure.filter = Some(FilterTree::Leaf(FilterLeaf {
            field: "state".to_owned(),
            op: FilterOp::Eq,
            value: Some(FilterValue::Scalar(Scalar::String(injection.to_owned()))),
        }));

        let compiled = compile(&measure, CacheRowKind::Aggregate);

        assert!(!compiled.sql.contains("DROP TABLE"), "{}", compiled.sql);
        assert_eq!(
            compiled
                .params
                .iter()
                .filter(|param| **param == text(injection))
                .count(),
            3
        );
    }

    #[test]
    fn every_placeholder_has_exactly_one_bound_parameter() {
        for kind in [
            CacheRowKind::Aggregate,
            CacheRowKind::Event,
            CacheRowKind::Subject,
        ] {
            let mut measure = measure();
            measure.aggregation = Aggregation::CountDistinct;
            measure.subject_expr = Some("pull_request_id".to_owned());
            measure.value_expr = None;
            if kind == CacheRowKind::Event {
                measure.aggregation = Aggregation::Sum;
                measure.value_expr = Some("lines_added".to_owned());
                measure.subject_expr = None;
            }

            let compiled = compile(&measure, kind);

            assert_eq!(
                compiled.sql.matches('?').count(),
                compiled.params.len(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_fold_missing_its_operand_is_rejected_rather_than_rendered() {
        let mut measure = measure();
        measure.aggregation = Aggregation::Sum;

        assert_eq!(
            compile_cache_build(
                &dataset(&measure.dataset),
                &build(&measure, CacheRowKind::Aggregate)
            )
            .expect_err("expected a compile error"),
            CompileError::MissingOperand {
                measure: "prs_merged".to_owned(),
                aggregation: "sum",
                operand: "value",
            }
        );
    }

    #[test]
    fn a_measure_a_distribution_reads_is_kept_per_event() {
        let mut measure = measure();
        measure.aggregation = Aggregation::Sum;
        measure.value_expr = Some("lines_added".to_owned());

        for computation in [
            Computation::Percentile {
                measure: measure.key.clone(),
                quantile: 0.5,
            },
            Computation::Stddev {
                measure: measure.key.clone(),
            },
        ] {
            let metrics = vec![metric("git.pr_size", computation)];

            assert_eq!(row_kind(&measure, &metrics), CacheRowKind::Event);
        }
    }

    #[test]
    fn an_average_keeps_its_rows_even_when_no_distribution_reads_it() {
        let mut averaged = measure();
        averaged.aggregation = Aggregation::Avg;
        averaged.value_expr = Some("lines_added".to_owned());

        let metrics = vec![metric(
            "git.pr_size",
            Computation::Direct {
                measure: averaged.key.clone(),
            },
        )];

        assert_eq!(row_kind(&averaged, &metrics), CacheRowKind::Event);
    }

    #[test]
    fn a_fold_a_wider_window_can_re_fold_is_kept_at_that_fold() {
        let counted = measure();
        let mut distinct = measure();
        distinct.aggregation = Aggregation::CountDistinct;
        distinct.subject_expr = Some("pull_request_id".to_owned());

        let metrics = vec![
            metric(
                "git.prs_merged",
                Computation::Direct {
                    measure: counted.key.clone(),
                },
            ),
            metric(
                "git.pr_size",
                Computation::Percentile {
                    measure: "another_measure".to_owned(),
                    quantile: 0.5,
                },
            ),
        ];

        assert_eq!(row_kind(&counted, &metrics), CacheRowKind::Aggregate);
        assert_eq!(row_kind(&distinct, &metrics), CacheRowKind::Subject);
    }
}
