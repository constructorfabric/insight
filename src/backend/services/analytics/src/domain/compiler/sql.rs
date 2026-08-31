//! The pieces every compiled read is assembled from: the relation to scan, the
//! fold, the time bucket, the scope predicates and a stored filter's tree.
//!
//! SAFETY: catalog-validated names, expressions and operators are written into
//! the statement; every value is bound.

use chrono::NaiveDate;

use crate::domain::definitions::definition::{
    Aggregation, DimensionBinding, MeasureDefinition, Operand,
};
use crate::domain::definitions::filter::{
    AllNode, AnyNode, FilterError, FilterLeaf, FilterOp, FilterTree, FilterValue, NotNode, Scalar,
};
use crate::domain::field_catalog::model::{CatalogDataset, FieldRole, ReadDiscipline};

use super::error::CompileError;
use super::request::{
    Bucket, DimensionFilter, DrilldownQuery, EntityScope, GroupRankingQuery, MeasureQuery,
    MetricQuery,
};

/// A bound value in the spelling ClickHouse receives it in. None is ever
/// written into the SQL text.
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

/// What a request scopes a read to, borrowed so both compile paths bind the
/// same predicates in the same order.
pub(super) struct ReadScope<'a> {
    pub tenant_id: &'a str,
    pub entity_scope: &'a EntityScope,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub dimension_filters: &'a [DimensionFilter],
}

impl<'a> ReadScope<'a> {
    pub fn of_measure(query: &'a MeasureQuery) -> Self {
        Self {
            tenant_id: &query.tenant_id,
            entity_scope: &query.entity_scope,
            from: query.from,
            to: query.to,
            dimension_filters: &query.dimension_filters,
        }
    }

    pub fn of_metric(query: &'a MetricQuery) -> Self {
        Self {
            tenant_id: &query.tenant_id,
            entity_scope: &query.entity_scope,
            from: query.from,
            to: query.to,
            dimension_filters: &query.dimension_filters,
        }
    }

    pub fn of_drilldown(query: &'a DrilldownQuery) -> Self {
        Self {
            tenant_id: &query.tenant_id,
            entity_scope: &query.entity_scope,
            from: query.from,
            to: query.to,
            dimension_filters: &query.dimension_filters,
        }
    }

    pub fn of_ranking(query: &'a GroupRankingQuery) -> Self {
        Self {
            tenant_id: &query.tenant_id,
            entity_scope: &query.entity_scope,
            from: query.from,
            to: query.to,
            dimension_filters: &query.dimension_filters,
        }
    }

    /// The same scope without the entity narrowing, for a read that joins a pool.
    pub fn over_every_entity(&self, population: &'a EntityScope) -> Self {
        Self {
            tenant_id: self.tenant_id,
            entity_scope: population,
            from: self.from,
            to: self.to,
            dimension_filters: self.dimension_filters,
        }
    }
}

pub(super) fn from_clause(dataset: &CatalogDataset) -> String {
    let relation = format!("{}.{}", dataset.database, dataset.relation);
    match dataset.read_discipline {
        ReadDiscipline::Collapsing => format!("{relation} FINAL"),
        ReadDiscipline::Direct => relation,
    }
}

// SAFETY: `value_expr` and `subject_expr` are written verbatim; both passed the
// scalar-expression allowlist and catalog binding before the definition stored.
pub(super) fn aggregate_expr(measure: &MeasureDefinition) -> Result<String, CompileError> {
    let function = aggregate_function(measure.aggregation);
    match measure.aggregation.operand() {
        Operand::None => Ok(format!("{function}()")),
        Operand::Value => Ok(format!("{function}({})", value_operand(measure)?)),
        Operand::Subject => Ok(format!("{function}({})", subject_operand(measure)?)),
    }
}

/// What a conditional fold reports when no scanned row satisfies it.
#[derive(Debug, Clone, Copy)]
pub(super) enum EmptyFold {
    /// The fold's own zero, which for a count or a sum is a real observation.
    Zero,
    /// NULL, for a fold whose zero would state something the rows do not.
    Null,
}

/// The same fold restricted to the rows one condition selects, so two measures
/// over one dataset can be folded in a single scan.
pub(super) fn conditional_aggregate_expr(
    measure: &MeasureDefinition,
    condition: &str,
    empty: EmptyFold,
) -> Result<String, CompileError> {
    let function = aggregate_function(measure.aggregation);
    let combinators = match empty {
        EmptyFold::Zero => "If",
        EmptyFold::Null => "IfOrNull",
    };
    match measure.aggregation.operand() {
        Operand::None => Ok(format!("{function}{combinators}({condition})")),
        Operand::Value => Ok(format!(
            "{function}{combinators}({}, {condition})",
            value_operand(measure)?
        )),
        Operand::Subject => Ok(format!(
            "{function}{combinators}({}, {condition})",
            subject_operand(measure)?
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

pub(super) fn value_operand(measure: &MeasureDefinition) -> Result<&str, CompileError> {
    require_operand(measure, measure.value_expr.as_deref(), "value")
}

pub(super) fn subject_operand(measure: &MeasureDefinition) -> Result<&str, CompileError> {
    require_operand(measure, measure.subject_expr.as_deref(), "subject")
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

pub(super) fn bucket_expr(event_time: &str, bucket: Bucket) -> String {
    match bucket {
        Bucket::Day => format!("toDate({event_time})"),
        Bucket::Week => format!("toStartOfWeek(toDate({event_time}), 1)"),
        Bucket::Month => format!("toStartOfMonth(toDate({event_time}))"),
    }
}

/// The `WHERE` predicates of a read over `measure`'s dataset, in binding order.
/// A read keeping two measures' filters apart passes `filter` as `None`.
pub(super) fn read_predicates(
    dataset: &CatalogDataset,
    measure: &MeasureDefinition,
    filter: Option<&FilterTree>,
    scope: &ReadScope<'_>,
    params: &mut Vec<QueryParam>,
) -> Result<Vec<String>, CompileError> {
    // INVARIANT: tenancy leads every read, bound from the request's resolved
    // tenant and never written into the SQL.
    let tenant_field = dataset
        .fields_with_role(FieldRole::Tenant)
        .next()
        .ok_or_else(|| CompileError::NoTenantField {
            dataset: dataset.key.clone(),
        })?;
    let mut predicates = vec![format!("{} = ?", tenant_field.name)];
    params.push(QueryParam::Text(scope.tenant_id.to_owned()));

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
                "{} IN ({})",
                measure.entity,
                placeholders(identities.len())
            ));
            params.extend(identities.iter().cloned().map(QueryParam::Text));
        }
    }

    let event_date = format!("toDate({})", measure.event_time);
    predicates.push(format!("{event_date} >= toDate(?)"));
    params.push(QueryParam::Text(scope.from.to_string()));
    predicates.push(format!("{event_date} <= toDate(?)"));
    params.push(QueryParam::Text(scope.to.to_string()));

    if let Some(filter) = filter {
        predicates.push(render_filter(measure, filter, params)?);
    }

    for filter in scope.dimension_filters {
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

pub(super) fn render_filter(
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

// INVARIANT: `FilterTree::validate` rejects an empty combinator at write time,
// so rendering its identity here invents no predicate.
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

pub(super) fn dimension_binding<'a>(
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

pub(super) fn label_field(binding: &DimensionBinding) -> &str {
    binding
        .label_field
        .as_deref()
        .unwrap_or(&binding.value_field)
}

pub(super) fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}
