//! Renders one measure-level aggregate read over its dataset.
//!
//! Field names, expressions, and operators come from a definition that already
//! passed catalog validation and are written into the statement; every value —
//! request-supplied or carried by the stored filter — is bound. That split is
//! the whole safety story, and the tests hold it.

use std::fmt::Write;

use crate::domain::definitions::definition::{
    Aggregation, DimensionBinding, MeasureDefinition, Operand,
};
use crate::domain::definitions::filter::{
    AllNode, AnyNode, FilterError, FilterLeaf, FilterOp, FilterTree, FilterValue, NotNode, Scalar,
};
use crate::domain::field_catalog::model::{CatalogDataset, FieldRole, ReadDiscipline};

use super::error::CompileError;
use super::request::{Bucket, EntityScope, MeasureQuery};

/// A bound value in the spelling ClickHouse receives it in. Every value a
/// statement carries is one of these, and none of them is ever written into
/// the SQL text.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryParam {
    Text(String),
    Int(i64),
    UInt(u64),
    Float(f64),
    /// ClickHouse spells a boolean as `UInt8`.
    Bool(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledMeasureQuery {
    pub sql: String,
    pub params: Vec<QueryParam>,
}

pub fn compile_measure_query(
    dataset: &CatalogDataset,
    measure: &MeasureDefinition,
    query: &MeasureQuery,
) -> Result<CompiledMeasureQuery, CompileError> {
    let group_by = query
        .group_by
        .as_deref()
        .map(|key| dimension_binding(measure, key))
        .transpose()?;
    let aggregate = aggregate_expr(measure)?;

    let mut params = Vec::new();
    let predicates = where_predicates(dataset, measure, query, &mut params)?;
    params.push(QueryParam::UInt(query.row_limit));

    let mut sql = String::from("SELECT\n");
    let _ = writeln!(sql, "    {} AS entity_id,", measure.entity);
    let _ = writeln!(
        sql,
        "    {} AS metric_date,",
        bucket_expr(&measure.event_time, query.bucket)
    );
    if let Some(binding) = group_by {
        let _ = writeln!(sql, "    {} AS dimension_value,", binding.value_field);
        let _ = writeln!(sql, "    {} AS dimension_label,", label_field(binding));
    }
    let _ = writeln!(sql, "    toFloat64({aggregate}) AS value");
    let _ = writeln!(sql, "FROM {}", from_clause(dataset));
    let _ = writeln!(sql, "WHERE {}", predicates.join("\n  AND "));
    let _ = writeln!(sql, "GROUP BY {}", group_columns(group_by).join(", "));
    let _ = write!(sql, "LIMIT ?");

    Ok(CompiledMeasureQuery { sql, params })
}

fn from_clause(dataset: &CatalogDataset) -> String {
    let relation = format!("{}.{}", dataset.database, dataset.relation);
    match dataset.read_discipline {
        ReadDiscipline::Collapsing => format!("{relation} FINAL"),
        ReadDiscipline::Direct => relation,
    }
}

// INVARIANT: `value_expr` and `subject_expr` are written into the statement
// verbatim. Both are admitted only by the scalar-expression allowlist and are
// bound to catalogued columns before a definition can be stored, so the
// compiler never re-parses them.
fn aggregate_expr(measure: &MeasureDefinition) -> Result<String, CompileError> {
    let function = aggregate_function(measure.aggregation);
    match measure.aggregation.operand() {
        Operand::None => Ok(format!("{function}()")),
        Operand::Value => Ok(format!(
            "{function}({})",
            require_operand(measure, measure.value_expr.as_deref(), "value")?
        )),
        Operand::Subject => Ok(format!(
            "{function}({})",
            require_operand(measure, measure.subject_expr.as_deref(), "subject")?
        )),
    }
}

fn aggregate_function(aggregation: Aggregation) -> &'static str {
    match aggregation {
        Aggregation::Count => "count",
        Aggregation::Sum => "sum",
        Aggregation::Avg => "avg",
        Aggregation::Min => "min",
        Aggregation::Max => "max",
        Aggregation::CountDistinct => "uniqExact",
    }
}

fn require_operand<'a>(
    measure: &MeasureDefinition,
    expression: Option<&'a str>,
    operand: &'static str,
) -> Result<&'a str, CompileError> {
    expression.ok_or_else(|| CompileError::MissingOperand {
        measure: measure.key.clone(),
        aggregation: measure.aggregation.as_db(),
        operand,
    })
}

fn bucket_expr(event_time: &str, bucket: Bucket) -> String {
    match bucket {
        Bucket::Day => format!("toDate({event_time})"),
        Bucket::Week => format!("toStartOfWeek(toDate({event_time}), 1)"),
        Bucket::Month => format!("toStartOfMonth(toDate({event_time}))"),
    }
}

fn where_predicates(
    dataset: &CatalogDataset,
    measure: &MeasureDefinition,
    query: &MeasureQuery,
    params: &mut Vec<QueryParam>,
) -> Result<Vec<String>, CompileError> {
    // INVARIANT: tenancy leads every read and is always enforced. The value is
    // bound from the request's resolved tenant, never written into the SQL.
    let tenant_field = dataset
        .fields_with_role(FieldRole::Tenant)
        .next()
        .ok_or_else(|| CompileError::NoTenantField {
            dataset: dataset.key.clone(),
        })?;
    let mut predicates = vec![format!("{} = ?", tenant_field.name)];
    params.push(QueryParam::Text(query.tenant_id.clone()));

    match &query.entity_scope {
        EntityScope::Tenant => {}
        EntityScope::Identities(identities) => {
            if identities.is_empty() {
                return Err(CompileError::EmptySelection {
                    selection: "the entity scope".to_owned(),
                });
            }
            predicates.push(format!(
                "{} IN ({})",
                measure.entity,
                placeholders(identities.len())
            ));
            params.extend(identities.iter().cloned().map(QueryParam::Text));
        }
    }

    let event_date = format!("toDate({})", measure.event_time);
    predicates.push(format!("{event_date} >= toDate(?)"));
    params.push(QueryParam::Text(query.from.to_string()));
    predicates.push(format!("{event_date} <= toDate(?)"));
    params.push(QueryParam::Text(query.to.to_string()));

    if let Some(filter) = &measure.filter {
        predicates.push(render_filter(measure, filter, params)?);
    }

    for filter in &query.dimension_filters {
        let binding = dimension_binding(measure, &filter.key)?;
        if filter.values.is_empty() {
            return Err(CompileError::EmptySelection {
                selection: format!("dimension filter `{}`", filter.key),
            });
        }
        predicates.push(format!(
            "{} IN ({})",
            binding.value_field,
            placeholders(filter.values.len())
        ));
        params.extend(filter.values.iter().cloned().map(QueryParam::Text));
    }

    Ok(predicates)
}

fn render_filter(
    measure: &MeasureDefinition,
    tree: &FilterTree,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    match tree {
        FilterTree::All(AllNode { all }) => render_combinator(measure, all, "AND", "1", params),
        FilterTree::Any(AnyNode { any }) => render_combinator(measure, any, "OR", "0", params),
        FilterTree::Not(NotNode { not }) => {
            Ok(format!("NOT ({})", render_filter(measure, not, params)?))
        }
        FilterTree::Leaf(leaf) => render_leaf(measure, leaf, params),
    }
}

// INVARIANT: `FilterTree::validate` rejects an empty combinator at write time.
// Rendering its identity keeps this function total without inventing a
// predicate the definition never expressed.
fn render_combinator(
    measure: &MeasureDefinition,
    children: &[FilterTree],
    operator: &str,
    identity: &str,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    if children.is_empty() {
        return Ok(identity.to_owned());
    }

    let mut rendered = Vec::with_capacity(children.len());
    for child in children {
        rendered.push(render_filter(measure, child, params)?);
    }

    Ok(format!("({})", rendered.join(&format!(" {operator} "))))
}

/// The three shapes a leaf takes in SQL, each with the value arity it accepts.
enum LeafForm {
    Comparison(&'static str),
    Membership(&'static str),
    NullCheck(&'static str),
}

fn leaf_form(op: FilterOp) -> LeafForm {
    match op {
        FilterOp::Eq => LeafForm::Comparison("="),
        FilterOp::Neq => LeafForm::Comparison("!="),
        FilterOp::Gt => LeafForm::Comparison(">"),
        FilterOp::Gte => LeafForm::Comparison(">="),
        FilterOp::Lt => LeafForm::Comparison("<"),
        FilterOp::Lte => LeafForm::Comparison("<="),
        FilterOp::In => LeafForm::Membership("IN"),
        FilterOp::NotIn => LeafForm::Membership("NOT IN"),
        FilterOp::IsNull => LeafForm::NullCheck("IS NULL"),
        FilterOp::NotNull => LeafForm::NullCheck("IS NOT NULL"),
    }
}

fn render_leaf(
    measure: &MeasureDefinition,
    leaf: &FilterLeaf,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    let malformed = |source: FilterError| CompileError::MalformedFilter {
        measure: measure.key.clone(),
        field: leaf.field.clone(),
        source,
    };

    match leaf_form(leaf.op) {
        LeafForm::Comparison(operator) => match &leaf.value {
            Some(FilterValue::Scalar(scalar)) => {
                params.push(scalar_param(measure, &leaf.field, scalar)?);
                Ok(format!("{} {operator} ?", leaf.field))
            }
            Some(FilterValue::List(_)) | None => Err(malformed(FilterError::ScalarValueRequired)),
        },
        LeafForm::Membership(operator) => match &leaf.value {
            Some(FilterValue::List(items)) if !items.is_empty() => {
                for item in items {
                    params.push(scalar_param(measure, &leaf.field, item)?);
                }
                Ok(format!(
                    "{} {operator} ({})",
                    leaf.field,
                    placeholders(items.len())
                ))
            }
            Some(FilterValue::List(_) | FilterValue::Scalar(_)) | None => {
                Err(malformed(FilterError::ListValueRequired))
            }
        },
        LeafForm::NullCheck(operator) => match &leaf.value {
            None => Ok(format!("{} {operator}", leaf.field)),
            Some(FilterValue::Scalar(_) | FilterValue::List(_)) => {
                Err(malformed(FilterError::ValueNotAllowed))
            }
        },
    }
}

fn scalar_param(
    measure: &MeasureDefinition,
    field: &str,
    scalar: &Scalar,
) -> Result<QueryParam, CompileError> {
    match scalar {
        Scalar::Bool(value) => Ok(QueryParam::Bool(u8::from(*value))),
        Scalar::String(value) => Ok(QueryParam::Text(value.clone())),
        Scalar::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(QueryParam::Int(value))
            } else if let Some(value) = number.as_u64() {
                Ok(QueryParam::UInt(value))
            } else if let Some(value) = number.as_f64() {
                Ok(QueryParam::Float(value))
            } else {
                Err(CompileError::UnbindableNumber {
                    measure: measure.key.clone(),
                    field: field.to_owned(),
                    value: number.to_string(),
                })
            }
        }
    }
}

fn dimension_binding<'a>(
    measure: &'a MeasureDefinition,
    key: &str,
) -> Result<&'a DimensionBinding, CompileError> {
    measure
        .dimensions
        .iter()
        .find(|binding| binding.key == key)
        .ok_or_else(|| CompileError::UnknownDimension {
            measure: measure.key.clone(),
            key: key.to_owned(),
        })
}

fn label_field(binding: &DimensionBinding) -> &str {
    binding
        .label_field
        .as_deref()
        .unwrap_or(&binding.value_field)
}

fn group_columns(group_by: Option<&DimensionBinding>) -> Vec<&'static str> {
    match group_by {
        Some(_) => vec![
            "entity_id",
            "metric_date",
            "dimension_value",
            "dimension_label",
        ],
        None => vec!["entity_id", "metric_date"],
    }
}

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::domain::compiler::request::DimensionFilter;
    use crate::domain::definitions::definition::Aggregation;
    use crate::domain::field_catalog::loader;
    use crate::domain::field_catalog::model::{CatalogField, FieldCatalog, FieldType};

    const SNAPSHOT: &str = r#"[
      {
        "database": "silver",
        "relation": "class_git_pull_requests",
        "engine": "ReplacingMergeTree",
        "sorting_key": "unique_key",
        "columns": [
          {"name": "tenant_id", "type": "Nullable(String)"},
          {"name": "author_email", "type": "String"},
          {"name": "closed_on", "type": "Nullable(DateTime)"},
          {"name": "state", "type": "String"},
          {"name": "repo_slug", "type": "String"},
          {"name": "data_source", "type": "String"},
          {"name": "data_source_label", "type": "String"},
          {"name": "is_draft", "type": "Bool"},
          {"name": "lines_added", "type": "Nullable(Int64)"},
          {"name": "pull_request_id", "type": "String"}
        ]
      },
      {
        "database": "silver",
        "relation": "class_git_commits",
        "engine": "MergeTree",
        "sorting_key": "unique_key",
        "columns": [
          {"name": "tenant_id", "type": "Nullable(String)"},
          {"name": "author_email", "type": "String"},
          {"name": "committed_on", "type": "Nullable(DateTime)"},
          {"name": "repo_slug", "type": "String"}
        ]
      }
    ]"#;

    const ROLES: &str = "
datasets:
  - key: git_pull_requests
    database: silver
    relation: class_git_pull_requests
    fields:
      tenant_id: tenant
      author_email: entity
      closed_on: event_time
      state: dimension
      repo_slug: dimension
      data_source: dimension
      is_draft: dimension
      lines_added: measurable
      pull_request_id: dimension
  - key: git_commits
    database: silver
    relation: class_git_commits
    fields:
      tenant_id: tenant
      author_email: entity
      committed_on: event_time
      repo_slug: dimension
";

    fn catalog() -> FieldCatalog {
        loader::load(SNAPSHOT, ROLES).expect("catalog loads")
    }

    fn dataset(key: &str) -> CatalogDataset {
        catalog()
            .dataset(key)
            .expect("dataset is catalogued")
            .clone()
    }

    fn measure() -> MeasureDefinition {
        MeasureDefinition {
            key: "prs_merged".to_owned(),
            dataset: "git_pull_requests".to_owned(),
            description: None,
            filter: serde_yaml::from_str(
                "
all:
  - { field: state, op: eq, value: merged }
  - { field: lines_added, op: gte, value: 500 }
",
            )
            .ok(),
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

    fn query() -> MeasureQuery {
        MeasureQuery {
            tenant_id: "acme-tenant".to_owned(),
            entity_scope: EntityScope::Tenant,
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date"),
            bucket: Bucket::Day,
            dimension_filters: Vec::new(),
            group_by: None,
            row_limit: 10_001,
        }
    }

    fn text(value: &str) -> QueryParam {
        QueryParam::Text(value.to_owned())
    }

    fn lines(expected: &[&str]) -> String {
        expected.join("\n")
    }

    fn compile(measure: &MeasureDefinition, query: &MeasureQuery) -> CompiledMeasureQuery {
        compile_measure_query(&dataset(&measure.dataset), measure, query).expect("compiles")
    }

    fn compile_err(measure: &MeasureDefinition, query: &MeasureQuery) -> CompileError {
        compile_measure_query(&dataset(&measure.dataset), measure, query)
            .expect_err("expected a compile error")
    }

    #[test]
    fn a_count_over_identities_reads_the_dataset_at_the_requested_grain() {
        let mut query = query();
        query.bucket = Bucket::Week;
        query.entity_scope = EntityScope::Identities(vec!["dev@example.com".to_owned()]);
        query.dimension_filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: vec!["example/app".to_owned()],
        }];

        let compiled = compile(&measure(), &query);

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    author_email AS entity_id,",
                "    toStartOfWeek(toDate(closed_on), 1) AS metric_date,",
                "    toFloat64(count()) AS value",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND author_email IN (?)",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "  AND (state = ? AND lines_added >= ?)",
                "  AND repo_slug IN (?)",
                "GROUP BY entity_id, metric_date",
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
                text("merged"),
                QueryParam::Int(500),
                text("example/app"),
                QueryParam::UInt(10_001),
            ]
        );
    }

    #[test]
    fn a_tenant_scope_adds_no_entity_predicate_beyond_tenancy() {
        let mut measure = measure();
        measure.filter = None;

        let compiled = compile(&measure, &query());

        assert!(compiled.sql.contains("WHERE tenant_id = ?\n  AND toDate("));
        assert!(!compiled.sql.contains("author_email IN"));
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
    fn every_aggregation_folds_its_own_operand() {
        let cases = [
            (
                Aggregation::Count,
                None,
                None,
                "toFloat64(count()) AS value",
            ),
            (
                Aggregation::Sum,
                Some("lines_added"),
                None,
                "toFloat64(sum(lines_added)) AS value",
            ),
            (
                Aggregation::Avg,
                Some("lines_added"),
                None,
                "toFloat64(avg(lines_added)) AS value",
            ),
            (
                Aggregation::Min,
                Some("lines_added"),
                None,
                "toFloat64(min(lines_added)) AS value",
            ),
            (
                Aggregation::Max,
                Some("lines_added"),
                None,
                "toFloat64(max(lines_added)) AS value",
            ),
            (
                Aggregation::CountDistinct,
                None,
                Some("pull_request_id"),
                "toFloat64(uniqExact(pull_request_id)) AS value",
            ),
        ];

        for (aggregation, value_expr, subject_expr, expected) in cases {
            let mut measure = measure();
            measure.aggregation = aggregation;
            measure.value_expr = value_expr.map(str::to_owned);
            measure.subject_expr = subject_expr.map(str::to_owned);

            let compiled = compile(&measure, &query());

            assert!(
                compiled.sql.contains(expected),
                "{aggregation:?}: {}",
                compiled.sql
            );
        }
    }

    #[test]
    fn a_validated_value_expression_is_embedded_verbatim() {
        let mut measure = measure();
        measure.aggregation = Aggregation::Sum;
        measure.value_expr = Some("lines_added + 1".to_owned());

        assert!(
            compile(&measure, &query())
                .sql
                .contains("toFloat64(sum(lines_added + 1)) AS value")
        );
    }

    #[test]
    fn an_aggregation_missing_its_operand_is_rejected() {
        let mut measure = measure();
        measure.aggregation = Aggregation::Sum;

        assert_eq!(
            compile_err(&measure, &query()),
            CompileError::MissingOperand {
                measure: "prs_merged".to_owned(),
                aggregation: "sum",
                operand: "value",
            }
        );
    }

    #[test]
    fn each_bucket_renders_its_own_start_function() {
        let cases = [
            (Bucket::Day, "    toDate(closed_on) AS metric_date,"),
            (
                Bucket::Week,
                "    toStartOfWeek(toDate(closed_on), 1) AS metric_date,",
            ),
            (
                Bucket::Month,
                "    toStartOfMonth(toDate(closed_on)) AS metric_date,",
            ),
        ];

        for (bucket, expected) in cases {
            let mut query = query();
            query.bucket = bucket;

            assert!(
                compile(&measure(), &query).sql.contains(expected),
                "{bucket:?}"
            );
        }
    }

    #[test]
    fn nested_combinators_render_as_parenthesized_predicates() {
        let mut measure = measure();
        measure.filter = serde_yaml::from_str(
            "
any:
  - not:
      { field: closed_on, op: not_null }
  - { field: state, op: in, value: [open, draft] }
  - { field: is_draft, op: eq, value: false }
",
        )
        .ok();

        let compiled = compile(&measure, &query());

        assert!(
            compiled
                .sql
                .contains("  AND (NOT (closed_on IS NOT NULL) OR state IN (?, ?) OR is_draft = ?)")
        );
        assert_eq!(
            compiled.params,
            vec![
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                text("open"),
                text("draft"),
                QueryParam::Bool(0),
                QueryParam::UInt(10_001),
            ]
        );
    }

    #[test]
    fn scalar_kinds_bind_in_their_clickhouse_spelling() {
        let cases = [
            ("{ field: state, op: eq, value: merged }", text("merged")),
            (
                "{ field: lines_added, op: eq, value: 500 }",
                QueryParam::Int(500),
            ),
            (
                "{ field: lines_added, op: eq, value: -7 }",
                QueryParam::Int(-7),
            ),
            (
                "{ field: lines_added, op: eq, value: 1.5 }",
                QueryParam::Float(1.5),
            ),
            (
                "{ field: is_draft, op: eq, value: true }",
                QueryParam::Bool(1),
            ),
        ];

        for (filter, expected) in cases {
            let mut measure = measure();
            measure.filter = serde_yaml::from_str(filter).ok();

            let compiled = compile(&measure, &query());

            assert_eq!(compiled.params[3], expected, "{filter}");
        }
    }

    #[test]
    fn a_grouped_dimension_selects_its_value_and_label() {
        let mut labelled = query();
        labelled.group_by = Some("source".to_owned());

        let compiled = compile(&measure(), &labelled);

        assert!(compiled.sql.contains("    data_source AS dimension_value,"));
        assert!(
            compiled
                .sql
                .contains("    data_source_label AS dimension_label,")
        );
        assert!(
            compiled
                .sql
                .contains("GROUP BY entity_id, metric_date, dimension_value, dimension_label")
        );

        let mut unlabelled = query();
        unlabelled.group_by = Some("repository".to_owned());

        let compiled = compile(&measure(), &unlabelled);

        assert!(compiled.sql.contains("    repo_slug AS dimension_value,"));
        assert!(compiled.sql.contains("    repo_slug AS dimension_label,"));
    }

    #[test]
    fn final_appears_only_for_a_collapsing_dataset() {
        let mut measure = measure();
        measure.filter = None;
        assert!(
            compile(&measure, &query())
                .sql
                .contains("FROM silver.class_git_pull_requests FINAL")
        );

        measure.dataset = "git_commits".to_owned();
        measure.event_time = "committed_on".to_owned();
        measure.dimensions = Vec::new();
        let compiled = compile(&measure, &query());

        assert!(compiled.sql.contains("FROM silver.class_git_commits\n"));
        assert!(!compiled.sql.contains("FINAL"));
    }

    #[test]
    fn filter_values_never_reach_the_sql_text() {
        let injection = "'; DROP TABLE x; --";
        let mut measure = measure();
        measure.filter = Some(FilterTree::Leaf(FilterLeaf {
            field: "state".to_owned(),
            op: FilterOp::Eq,
            value: Some(FilterValue::Scalar(Scalar::String(injection.to_owned()))),
        }));
        let mut query = query();
        query.entity_scope = EntityScope::Identities(vec![injection.to_owned()]);
        query.dimension_filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: vec![injection.to_owned()],
        }];

        let compiled = compile(&measure, &query);

        assert!(!compiled.sql.contains("DROP TABLE"), "{}", compiled.sql);
        assert!(!compiled.sql.contains('\''), "{}", compiled.sql);
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
        let mut query = query();
        query.bucket = Bucket::Month;
        query.group_by = Some("source".to_owned());
        query.entity_scope =
            EntityScope::Identities(vec!["a@example.com".to_owned(), "b@example.com".to_owned()]);
        query.dimension_filters = vec![DimensionFilter {
            key: "source".to_owned(),
            values: vec!["github".to_owned(), "gitlab".to_owned()],
        }];

        let compiled = compile(&measure(), &query);

        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn a_dimension_filter_on_an_undeclared_key_is_rejected() {
        let mut query = query();
        query.dimension_filters = vec![DimensionFilter {
            key: "team".to_owned(),
            values: vec!["platform".to_owned()],
        }];

        assert_eq!(
            compile_err(&measure(), &query),
            CompileError::UnknownDimension {
                measure: "prs_merged".to_owned(),
                key: "team".to_owned(),
            }
        );
    }

    #[test]
    fn an_undeclared_group_by_dimension_is_rejected() {
        let mut query = query();
        query.group_by = Some("team".to_owned());

        assert_eq!(
            compile_err(&measure(), &query),
            CompileError::UnknownDimension {
                measure: "prs_merged".to_owned(),
                key: "team".to_owned(),
            }
        );
    }

    #[test]
    fn a_selection_with_no_values_is_rejected_rather_than_rendered() {
        let mut empty_entities = query();
        empty_entities.entity_scope = EntityScope::Identities(Vec::new());
        assert_eq!(
            compile_err(&measure(), &empty_entities),
            CompileError::EmptySelection {
                selection: "the entity scope".to_owned(),
            }
        );

        let mut empty_filter = query();
        empty_filter.dimension_filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: Vec::new(),
        }];
        assert_eq!(
            compile_err(&measure(), &empty_filter),
            CompileError::EmptySelection {
                selection: "dimension filter `repository`".to_owned(),
            }
        );
    }

    #[test]
    fn a_filter_whose_value_contradicts_its_operator_is_rejected() {
        let mut measure = measure();
        measure.filter = Some(FilterTree::Leaf(FilterLeaf {
            field: "state".to_owned(),
            op: FilterOp::In,
            value: Some(FilterValue::Scalar(Scalar::String("merged".to_owned()))),
        }));

        assert_eq!(
            compile_err(&measure, &query()),
            CompileError::MalformedFilter {
                measure: "prs_merged".to_owned(),
                field: "state".to_owned(),
                source: FilterError::ListValueRequired,
            }
        );
    }

    #[test]
    fn a_dataset_without_a_tenant_field_is_rejected() {
        let untenanted = CatalogDataset {
            key: "git_pull_requests".to_owned(),
            database: "silver".to_owned(),
            relation: "class_git_pull_requests".to_owned(),
            read_discipline: ReadDiscipline::Direct,
            sorting_key: vec!["unique_key".to_owned()],
            fields: vec![CatalogField {
                name: "author_email".to_owned(),
                field_type: FieldType::parse("String"),
                role: Some(FieldRole::Entity),
                display: Vec::new(),
                label_field: None,
            }],
        };

        assert_eq!(
            compile_measure_query(&untenanted, &measure(), &query())
                .expect_err("expected a compile error"),
            CompileError::NoTenantField {
                dataset: "git_pull_requests".to_owned(),
            }
        );
    }
}
