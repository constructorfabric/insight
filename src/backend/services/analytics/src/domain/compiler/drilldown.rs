//! Renders a metric's rows: the dataset rows an aggregate read folded,
//! projected one per row instead of one per group. INVARIANT: both shapes
//! reach the scan through [`read_predicates`], so what a page shows cannot
//! drift from what the value counted.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::domain::definitions::definition::{Aggregation, MeasureDefinition, MetricDefinition};
use crate::domain::field_catalog::model::{CatalogDataset, DisplayRole, FieldCatalog};

use super::dimensions::{dimension_aliases, dimension_label_expr, dimension_value_expr};
use super::error::CompileError;
use super::fold::Fold;
use super::pool::{Pool, joined_entity, only_cte, scan_clause};
use super::request::{DrilldownCursor, DrilldownQuery, DrilldownSort, SortDirection, SortValue};
use super::sql::{
    QueryParam, ReadScope, dimension_binding, from_clause, placeholders, read_predicates,
    subject_operand, value_operand,
};

/// The part an input plays in the metric's value, tagged on every row so a
/// composed metric's pages read back apart. A derived input is tagged by alias.
pub const ROLE_VALUE: &str = "value";
pub const ROLE_NUMERATOR: &str = "numerator";
pub const ROLE_DENOMINATOR: &str = "denominator";

const ENTITY_ID: &str = "entity_id";
const INPUT_ROLE: &str = "input_role";
const METRIC_DATE: &str = "metric_date";
const OBSERVED_AT: &str = "observed_at";
const CONTRIBUTION: &str = "contribution";
const SUBJECT: &str = "subject";

/// Every display role in the order a row projects them, so the column list of
/// a dataset is decided by the catalog rather than by field order.
const DISPLAY_ROLES: [DisplayRole; 5] = [
    DisplayRole::Title,
    DisplayRole::Reference,
    DisplayRole::Actor,
    DisplayRole::Location,
    DisplayRole::Link,
];

/// What a projected column is, so a reader decodes a page by meaning rather
/// than by matching alias spellings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrilldownColumnKind {
    EntityId,
    InputRole,
    Date,
    ObservedAt,
    Contribution,
    /// The distinct value one row of a `count_distinct` page stands for.
    Subject,
    Display(DisplayRole),
    DimensionValue(String),
    DimensionLabel(String),
    /// A component of the keyset position, at its place in the sort order.
    SortKey(usize),
}

impl DrilldownColumnKind {
    /// The name a page reports this column under, absent when the column is not
    /// reported at all.
    ///
    /// Three kinds are reported as none, each because the page already says it
    /// better elsewhere: the input role, which the answer names once beside the
    /// rows; the ordering values, which the cursor carries; and the
    /// contribution of a row that was itself what the fold counted. A dimension
    /// is reported twice — its label under the dimension's own key, because
    /// that is what a reader sees, and its value under a suffixed one, because
    /// that is what the metric grouped by.
    ///
    /// INVARIANT: this is the only naming of a reported column, so what a page
    /// may be ordered by cannot drift from what it shows.
    pub fn reported_key(&self, contribution: Contribution) -> Option<String> {
        match self {
            Self::EntityId => Some("subject".to_owned()),
            Self::InputRole | Self::SortKey(_) => None,
            Self::Date => Some("date".to_owned()),
            Self::ObservedAt => Some("observed_at".to_owned()),
            Self::Contribution => match contribution {
                Contribution::CountedRow => None,
                Contribution::MeasuredValue => Some("value".to_owned()),
            },
            Self::Subject => Some("subject_key".to_owned()),
            Self::Display(role) => Some(display_key(*role)),
            Self::DimensionLabel(key) => Some(key.clone()),
            Self::DimensionValue(key) => Some(format!("{key}_value")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrilldownColumn {
    pub alias: String,
    pub kind: DrilldownColumnKind,
}

/// The relation a page scanned, so a caller can bind a page to the table its
/// rows came from without resolving the metric a second time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedRelation {
    pub database: String,
    pub relation: String,
}

/// What one row contributes to the metric's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contribution {
    /// Every row contributes the same 1, because the row itself is what the
    /// fold counted. Reporting that number says nothing the row does not.
    CountedRow,
    /// Each row contributes the value the fold read from it.
    MeasuredValue,
}

impl Contribution {
    fn of(aggregation: Aggregation) -> Self {
        match aggregation {
            Aggregation::Count | Aggregation::CountDistinct => Self::CountedRow,
            Aggregation::Sum | Aggregation::Avg | Aggregation::Min | Aggregation::Max => {
                Self::MeasuredValue
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledDrilldown {
    pub input_role: String,
    pub relation: ScannedRelation,
    pub contribution: Contribution,
    pub sql: String,
    pub params: Vec<QueryParam>,
    pub columns: Vec<DrilldownColumn>,
}

/// The parts a metric's computation is composed of, named exactly as the pages
/// of its rows are tagged.
pub fn drilldown_input_roles(
    metric: &MetricDefinition,
    measures: &BTreeMap<String, MeasureDefinition>,
) -> Result<Vec<String>, CompileError> {
    Ok(Fold::resolve(metric, measures)?
        .inputs()
        .into_iter()
        .map(|(role, _)| role.to_owned())
        .collect())
}

/// One page of rows per input of the metric's computation: one for a metric
/// over a single measure, one per composed input otherwise.
pub fn compile_drilldown(
    catalog: &FieldCatalog,
    metric: &MetricDefinition,
    measures: &BTreeMap<String, MeasureDefinition>,
    query: &DrilldownQuery,
) -> Result<Vec<CompiledDrilldown>, CompileError> {
    let fold = Fold::resolve(metric, measures)?;
    let dataset = fold.dataset(catalog)?;

    fold.inputs()
        .into_iter()
        .map(|(role, measure)| compile_drilldown_rows(dataset, measure, query, role))
        .collect()
}

/// Every column a page of each input reports, keyed by the input role: exactly
/// the set a request may ask that page to be ordered by.
pub fn drilldown_reported_columns(
    catalog: &FieldCatalog,
    metric: &MetricDefinition,
    measures: &BTreeMap<String, MeasureDefinition>,
    display_dimensions: &[String],
) -> Result<BTreeMap<String, Vec<String>>, CompileError> {
    let fold = Fold::resolve(metric, measures)?;
    let dataset = fold.dataset(catalog)?;

    fold.inputs()
        .into_iter()
        .map(|(role, measure)| {
            let grain = RowGrain::of(measure)?;
            let entity = joined_entity(None, &measure.entity);
            let sort_keys = grain.sort_exprs(dataset, entity)?;
            let projection = page_projection(
                dataset,
                measure,
                &grain,
                display_dimensions,
                entity,
                &sort_keys,
            )?;

            let contribution = Contribution::of(measure.aggregation);
            Ok((role.to_owned(), projection.reported_keys(contribution)))
        })
        .collect()
}

// INVARIANT: placeholders bind by position, so parameters are pushed in the
// order written: pool head, role tag, scope predicates, resume, page ceiling.
pub(super) fn compile_drilldown_rows(
    dataset: &CatalogDataset,
    measure: &MeasureDefinition,
    query: &DrilldownQuery,
    role: &str,
) -> Result<CompiledDrilldown, CompileError> {
    let grain = RowGrain::of(measure)?;
    let pool = Pool::of_scope(&query.entity_scope);
    let contribution = Contribution::of(measure.aggregation);

    let mut params = Vec::new();
    let head = only_cte(pool.as_ref(), &mut params)?;
    let entity = joined_entity(pool.as_ref(), &measure.entity);

    let sort_keys = grain.sort_exprs(dataset, entity)?;
    let projection = page_projection(
        dataset,
        measure,
        &grain,
        &query.display_dimensions,
        entity,
        &sort_keys,
    )?;
    params.push(QueryParam::Text(role.to_owned()));

    let ordering = Ordering::of(&projection, query.sort.as_ref(), contribution, &sort_keys)?;

    let mut predicates = read_predicates(
        dataset,
        measure,
        measure.filter.as_ref(),
        &ReadScope::of_drilldown(query),
        &mut params,
    )?;
    let mut folded_resume = None;
    if let Some(cursor) = &query.cursor {
        let resume = resume_predicate(cursor, &ordering, &sort_keys, &mut params)?;
        if grain.resumes_after_folding(&ordering) {
            folded_resume = Some(resume);
        } else {
            predicates.push(resume);
        }
    }
    params.push(QueryParam::UInt(query.page_size.saturating_add(1)));

    let mut sql = head;
    sql.push_str("SELECT\n");
    sql.push_str(&projection.select_list());
    let _ = writeln!(
        sql,
        "FROM {}",
        scan_clause(from_clause(dataset), pool.as_ref(), &measure.entity, "")
    );
    let _ = writeln!(sql, "WHERE {}", predicates.join("\n  AND "));
    if let Some(group) = grain.group_by() {
        let _ = writeln!(sql, "GROUP BY {group}");
    }
    if let Some(resume) = folded_resume {
        let _ = writeln!(sql, "HAVING {resume}");
    }
    let _ = writeln!(sql, "ORDER BY {}", ordering.order_by());
    let _ = write!(sql, "LIMIT ?");

    Ok(CompiledDrilldown {
        input_role: role.to_owned(),
        relation: ScannedRelation {
            database: dataset.database.clone(),
            relation: dataset.relation.clone(),
        },
        contribution,
        sql,
        params,
        columns: projection.columns(),
    })
}

fn page_projection(
    dataset: &CatalogDataset,
    measure: &MeasureDefinition,
    grain: &RowGrain,
    display_dimensions: &[String],
    entity: &str,
    sort_keys: &[String],
) -> Result<Projection, CompileError> {
    let mut projection = Projection::default();
    projection.push(ENTITY_ID, entity, DrilldownColumnKind::EntityId);
    projection.push(INPUT_ROLE, "?", DrilldownColumnKind::InputRole);

    grain.project_row(measure, &mut projection);
    project_displays(dataset, grain, &mut projection);
    project_dimensions(measure, display_dimensions, grain, &mut projection)?;

    for (index, expr) in sort_keys.iter().enumerate() {
        projection.push(sort_alias(index), expr, DrilldownColumnKind::SortKey(index));
    }

    Ok(projection)
}

/// What one row of a page stands for, which the measure's fold decides.
#[derive(Debug)]
enum RowGrain {
    /// One row per scanned event, carrying the contribution that event made.
    Event { contribution: String },
    /// One row per distinct subject, carrying the contribution it was credited.
    Subject { subject: String },
}

impl RowGrain {
    fn of(measure: &MeasureDefinition) -> Result<Self, CompileError> {
        match measure.aggregation {
            Aggregation::Count => Ok(Self::Event {
                contribution: "toFloat64(1)".to_owned(),
            }),
            Aggregation::Sum | Aggregation::Avg | Aggregation::Min | Aggregation::Max => {
                Ok(Self::Event {
                    contribution: format!("toFloat64({})", value_operand(measure)?),
                })
            }
            Aggregation::CountDistinct => Ok(Self::Subject {
                subject: subject_operand(measure)?.to_owned(),
            }),
        }
    }

    fn project_row(&self, measure: &MeasureDefinition, projection: &mut Projection) {
        let event_time = &measure.event_time;
        match self {
            Self::Event { contribution } => {
                projection.push(
                    METRIC_DATE,
                    &format!("toDate({event_time})"),
                    DrilldownColumnKind::Date,
                );
                projection.push(OBSERVED_AT, event_time, DrilldownColumnKind::ObservedAt);
                projection.push(
                    CONTRIBUTION,
                    contribution,
                    DrilldownColumnKind::Contribution,
                );
            }
            Self::Subject { subject } => {
                projection.push(SUBJECT, subject, DrilldownColumnKind::Subject);
                projection.push(
                    METRIC_DATE,
                    &format!("toDate(min({event_time}))"),
                    DrilldownColumnKind::Date,
                );
                projection.push(
                    OBSERVED_AT,
                    &format!("min({event_time})"),
                    DrilldownColumnKind::ObservedAt,
                );
                projection.push(
                    CONTRIBUTION,
                    "toFloat64(1)",
                    DrilldownColumnKind::Contribution,
                );
            }
        }
    }

    /// A subject row folds many events, so what they disagreed on is reported
    /// as a sorted set rather than as one arbitrary pick.
    fn collapse(&self, expr: &str) -> String {
        match self {
            Self::Event { .. } => expr.to_owned(),
            Self::Subject { .. } => {
                format!("arrayStringConcat(arraySort(groupUniqArray({expr})), ', ')")
            }
        }
    }

    fn group_by(&self) -> Option<String> {
        match self {
            Self::Event { .. } => None,
            Self::Subject { .. } => Some(format!("{ENTITY_ID}, {SUBJECT}")),
        }
    }

    /// A subject row's columns are folded from the events it collapsed, so a
    /// page ordered by one of them resumes after the grouping rather than on
    /// the scan, where that column does not exist yet.
    fn resumes_after_folding(&self, ordering: &Ordering) -> bool {
        match self {
            Self::Event { .. } => false,
            Self::Subject { .. } => ordering.sorted.is_some(),
        }
    }

    /// INVARIANT: what the page is ordered and resumed by is always row-level,
    /// so either shape resumes from a predicate on the scan, not on the result.
    fn sort_exprs(
        &self,
        dataset: &CatalogDataset,
        entity: &str,
    ) -> Result<Vec<String>, CompileError> {
        match self {
            Self::Event { .. } => Ok(page_order_columns(dataset)?
                .iter()
                .map(|column| as_text(column))
                .collect()),
            Self::Subject { subject } => Ok(vec![as_text(entity), as_text(subject)]),
        }
    }
}

/// One projected column: what it is read from, what it is called, and what it
/// means.
#[derive(Debug)]
struct Projected {
    alias: String,
    expr: String,
    kind: DrilldownColumnKind,
}

/// The projected columns and what each of them means, built together so the
/// select list and the column descriptions cannot disagree.
#[derive(Debug, Default)]
struct Projection {
    projected: Vec<Projected>,
}

impl Projection {
    fn push(&mut self, alias: impl Into<String>, expr: &str, kind: DrilldownColumnKind) {
        self.projected.push(Projected {
            alias: alias.into(),
            expr: expr.to_owned(),
            kind,
        });
    }

    fn select_list(&self) -> String {
        let exprs: Vec<String> = self
            .projected
            .iter()
            .map(|projected| format!("    {} AS {}", projected.expr, projected.alias))
            .collect();
        format!("{}\n", exprs.join(",\n"))
    }

    fn columns(&self) -> Vec<DrilldownColumn> {
        self.projected
            .iter()
            .map(|projected| DrilldownColumn {
                alias: projected.alias.clone(),
                kind: projected.kind.clone(),
            })
            .collect()
    }

    fn reported(&self, key: &str, contribution: Contribution) -> Option<&Projected> {
        self.projected
            .iter()
            .find(|projected| projected.kind.reported_key(contribution).as_deref() == Some(key))
    }

    fn reported_keys(&self, contribution: Contribution) -> Vec<String> {
        self.projected
            .iter()
            .filter_map(|projected| projected.kind.reported_key(contribution))
            .collect()
    }
}

// SAFETY: an alias written twice is not a statement, so a display role given
// to several fields projects the first of them and contributes one column.
fn project_displays(dataset: &CatalogDataset, grain: &RowGrain, projection: &mut Projection) {
    for role in DISPLAY_ROLES {
        let Some(field) = dataset
            .fields
            .iter()
            .find(|field| field.display.contains(&role))
        else {
            continue;
        };
        projection.push(
            display_alias(role),
            &grain.collapse(&as_text(&field.name)),
            DrilldownColumnKind::Display(role),
        );
    }
}

/// INVARIANT: a request may only name dimensions the measure declares, so a
/// page never projects a column the aggregate could not have grouped by.
fn project_dimensions(
    measure: &MeasureDefinition,
    display_dimensions: &[String],
    grain: &RowGrain,
    projection: &mut Projection,
) -> Result<(), CompileError> {
    let mut keys: Vec<&str> = measure
        .dimensions
        .iter()
        .map(|binding| binding.key.as_str())
        .collect();
    for requested in display_dimensions {
        dimension_binding(measure, requested)?;
        if !keys.contains(&requested.as_str()) {
            keys.push(requested);
        }
    }

    for (index, key) in keys.into_iter().enumerate() {
        let binding = dimension_binding(measure, key)?;
        let (value_alias, label_alias) = dimension_aliases(index);
        projection.push(
            value_alias,
            &grain.collapse(&dimension_value_expr(binding)),
            DrilldownColumnKind::DimensionValue(key.to_owned()),
        );
        projection.push(
            label_alias,
            &grain.collapse(&dimension_label_expr(binding)),
            DrilldownColumnKind::DimensionLabel(key.to_owned()),
        );
    }

    Ok(())
}

fn display_alias(role: DisplayRole) -> String {
    format!("display_{}", display_key(role))
}

fn display_key(role: DisplayRole) -> String {
    match role {
        DisplayRole::Title => "title",
        DisplayRole::Reference => "reference",
        DisplayRole::Actor => "actor",
        DisplayRole::Location => "location",
        DisplayRole::Link => "link",
    }
    .to_owned()
}

fn sort_alias(index: usize) -> String {
    format!("sort_{index}")
}

fn sort_order(count: usize) -> String {
    (0..count).map(sort_alias).collect::<Vec<_>>().join(", ")
}

/// The column a request asked a page to be ordered by, borrowed from the
/// projection that renders it.
#[derive(Debug)]
struct SortedColumn<'a> {
    alias: &'a str,
    expr: &'a str,
    direction: SortDirection,
}

/// How a page is ordered: the column a request asked for, ahead of the total
/// order every page falls back on.
#[derive(Debug)]
struct Ordering<'a> {
    sorted: Option<SortedColumn<'a>>,
    keys: usize,
}

impl<'a> Ordering<'a> {
    fn of(
        projection: &'a Projection,
        sort: Option<&DrilldownSort>,
        contribution: Contribution,
        sort_keys: &[String],
    ) -> Result<Self, CompileError> {
        let keys = sort_keys.len();

        let Some(sort) = sort else {
            return Ok(Self { sorted: None, keys });
        };
        let Some(projected) = projection.reported(&sort.column, contribution) else {
            return Err(CompileError::UnsortableColumn {
                column: sort.column.clone(),
                sortable: named_columns(&projection.reported_keys(contribution)),
            });
        };

        Ok(Self {
            sorted: Some(SortedColumn {
                alias: &projected.alias,
                expr: &projected.expr,
                direction: sort.direction,
            }),
            keys,
        })
    }

    /// INVARIANT: the total order always trails the requested column, so two
    /// rows tying on it still fall in one fixed order and a page never repeats
    /// or skips one.
    fn order_by(&self) -> String {
        let total = sort_order(self.keys);
        match &self.sorted {
            None => total,
            Some(sorted) => format!(
                "{} {} NULLS LAST, {total}",
                sorted.alias,
                sorted.direction.keyword()
            ),
        }
    }
}

/// The columns a page reports, as a refusal quotes them.
fn named_columns(reported: &[String]) -> String {
    reported
        .iter()
        .map(|key| format!("`{key}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// INVARIANT: the dataset's sorting key alone does not separate rows that tie
/// on it, so the row identity extends it into a total order.
fn page_order_columns(dataset: &CatalogDataset) -> Result<Vec<String>, CompileError> {
    let unorderable = |reason: &str| CompileError::UnorderableDataset {
        dataset: dataset.key.clone(),
        reason: reason.to_owned(),
    };

    if dataset.row_identity.is_empty() {
        return Err(unorderable(
            "it declares no row identity, so no order over its rows is provably total",
        ));
    }

    let mut columns = Vec::with_capacity(dataset.sorting_key.len() + dataset.row_identity.len());
    for part in dataset.sorting_key.iter().chain(&dataset.row_identity) {
        let Some(field) = dataset.field(part) else {
            return Err(CompileError::UnorderableDataset {
                dataset: dataset.key.clone(),
                reason: format!("it orders by `{part}`, which is not one of its columns"),
            });
        };
        if !columns.contains(&field.name) {
            columns.push(field.name.clone());
        }
    }

    Ok(columns)
}

/// INVARIANT: a page orders and resumes on the same expression, so a position
/// read off one page selects exactly the rows after it.
fn as_text(expr: &str) -> String {
    format!("ifNull(toString({expr}), '')")
}

/// INVARIANT: the expansion mirrors `ORDER BY <column> <direction> NULLS LAST`
/// arm by arm — one tuple compare can express neither a descending column nor
/// where the nulls sit.
fn resume_predicate(
    cursor: &DrilldownCursor,
    ordering: &Ordering,
    sort_keys: &[String],
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    if cursor.sort_values.len() != sort_keys.len() {
        return Err(CompileError::CursorArity {
            expected: sort_keys.len(),
            found: cursor.sort_values.len(),
        });
    }

    let Some(sorted) = &ordering.sorted else {
        return Ok(position_predicate(sort_keys, params, &cursor.sort_values));
    };

    let expr = sorted.expr;
    let Some(value) = &cursor.sort_value else {
        let position = position_predicate(sort_keys, params, &cursor.sort_values);
        return Ok(format!("({expr} IS NULL AND {position})"));
    };

    params.push(bound(value));
    params.push(bound(value));
    let after = sorted.direction.after();
    let position = position_predicate(sort_keys, params, &cursor.sort_values);

    Ok(format!(
        "({expr} {after} ? OR {expr} IS NULL OR ({expr} = ? AND {position}))"
    ))
}

/// The total order's own resume, which every page falls back on and a sorted
/// one breaks its ties with.
fn position_predicate(
    sort_keys: &[String],
    params: &mut Vec<QueryParam>,
    values: &[String],
) -> String {
    params.extend(values.iter().cloned().map(QueryParam::Text));

    format!(
        "tuple({}) > tuple({})",
        sort_keys.join(", "),
        placeholders(sort_keys.len())
    )
}

fn bound(value: &SortValue) -> QueryParam {
    match value {
        SortValue::Text(text) => QueryParam::Text(text.clone()),
        SortValue::Number(number) => QueryParam::Float(*number),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use chrono::NaiveDate;

    use crate::domain::compiler::error::CompileError;
    use crate::domain::compiler::fixtures::{
        derived, direct, drilldown_query, labelled_measure, lines, measure, measures, metric,
        people, people_params, percentile, pool_head, query, ratio, sized_measure, stddev, text,
    };
    use crate::domain::compiler::metric::compile_metric_query;
    use crate::domain::compiler::request::{
        DimensionFilter, DrilldownCursor, DrilldownQuery, DrilldownSort, EntityScope,
        SortDirection, SortValue, ViewKind,
    };
    use crate::domain::compiler::sql::QueryParam;
    use crate::domain::compiler::test_catalog::catalog;
    use crate::domain::definitions::definition::{
        Aggregation, MeasureDefinition, MetricDefinition,
    };
    use crate::domain::field_catalog::model::DisplayRole;

    use super::{
        CompiledDrilldown, Contribution, DrilldownColumnKind, ROLE_DENOMINATOR, ROLE_NUMERATOR,
        ROLE_VALUE, compile_drilldown, drilldown_input_roles, drilldown_reported_columns,
    };

    fn rows(
        metric: &MetricDefinition,
        defined: &[MeasureDefinition],
        query: &DrilldownQuery,
    ) -> Vec<CompiledDrilldown> {
        compile_drilldown(&catalog(), metric, &measures(defined), query).expect("compiles")
    }

    fn rows_err(
        metric: &MetricDefinition,
        defined: &[MeasureDefinition],
        query: &DrilldownQuery,
    ) -> CompileError {
        compile_drilldown(&catalog(), metric, &measures(defined), query)
            .expect_err("expected a compile error")
    }

    fn only(
        metric: &MetricDefinition,
        defined: &[MeasureDefinition],
        query: &DrilldownQuery,
    ) -> CompiledDrilldown {
        let mut compiled = rows(metric, defined, query);
        assert_eq!(compiled.len(), 1);
        compiled.remove(0)
    }

    fn where_block(sql: &str) -> String {
        sql.lines()
            .skip_while(|line| !line.starts_with("WHERE "))
            .take_while(|line| line.starts_with("WHERE ") || line.starts_with("  AND "))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn aliases(compiled: &CompiledDrilldown) -> Vec<&str> {
        compiled
            .columns
            .iter()
            .map(|column| column.alias.as_str())
            .collect()
    }

    fn counting_days() -> MeasureDefinition {
        MeasureDefinition {
            aggregation: Aggregation::CountDistinct,
            subject_expr: Some("toDate(closed_on)".to_owned()),
            ..measure("active_days", None)
        }
    }

    #[test]
    fn counting_rows_reports_every_scanned_row_with_the_contribution_it_made() {
        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure(
                "prs_merged",
                Some("{ field: state, op: eq, value: merged }"),
            )],
            &drilldown_query(),
        );

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    author_email AS entity_id,",
                "    ? AS input_role,",
                "    toDate(closed_on) AS metric_date,",
                "    closed_on AS observed_at,",
                "    toFloat64(1) AS contribution,",
                "    ifNull(toString(title), '') AS display_title,",
                "    ifNull(toString(pull_request_id), '') AS display_reference,",
                "    ifNull(toString(author_name), '') AS display_actor,",
                "    ifNull(toString(repo_slug), '') AS display_location,",
                "    coalesce(toString(repo_slug), '__unknown__') AS dim_0_value,",
                "    coalesce(toString(repo_slug), 'Unknown') AS dim_0_label,",
                "    ifNull(toString(unique_key), '') AS sort_0,",
                "    ifNull(toString(data_source), '') AS sort_1,",
                "    ifNull(toString(pull_request_id), '') AS sort_2",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "  AND state = ?",
                "ORDER BY sort_0, sort_1, sort_2",
                "LIMIT ?",
            ])
        );
        assert_eq!(
            compiled.params,
            vec![
                text("value"),
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                text("merged"),
                QueryParam::UInt(51),
            ]
        );
        assert_eq!(compiled.input_role, ROLE_VALUE);
    }

    #[test]
    fn a_page_adds_no_predicate_of_its_own_to_the_rows_the_aggregate_folded() {
        let compiled = only(
            &metric(direct("pr_size")),
            &[sized_measure("pr_size")],
            &drilldown_query(),
        );

        assert!(
            compiled
                .sql
                .contains("    toFloat64(lines_added) AS contribution,"),
            "{}",
            compiled.sql
        );
        assert!(!compiled.sql.contains("lines_added IS NOT NULL"));
        assert!(!compiled.sql.contains("lines_added >"));
    }

    #[test]
    fn a_page_scopes_its_scan_exactly_as_the_value_read_scopes_its_own() {
        let defined = [labelled_measure("prs_merged")];
        let scopes = [
            EntityScope::Tenant,
            EntityScope::Identities(vec!["dev@example.com".to_owned()]),
            people(),
        ];

        for scope in scopes {
            let mut aggregate = query(ViewKind::SubjectTotal);
            aggregate.entity_scope = scope.clone();
            aggregate.dimension_filters = vec![DimensionFilter {
                key: "source".to_owned(),
                values: vec!["github".to_owned()],
            }];

            let mut page = drilldown_query();
            page.entity_scope = scope;
            page.dimension_filters = aggregate.dimension_filters.clone();

            let value = compile_metric_query(
                &catalog(),
                &metric(direct("prs_merged")),
                &measures(&defined),
                &aggregate,
            )
            .expect("compiles");
            let compiled = only(&metric(direct("prs_merged")), &defined, &page);

            assert_eq!(where_block(&value.sql), where_block(&compiled.sql));
        }
    }

    #[test]
    fn counting_distinct_subjects_reports_one_row_per_subject_with_its_values_collapsed() {
        let compiled = only(
            &metric(direct("active_days")),
            &[counting_days()],
            &drilldown_query(),
        );

        assert_eq!(
            compiled.sql,
            lines(&[
                "SELECT",
                "    author_email AS entity_id,",
                "    ? AS input_role,",
                "    toDate(closed_on) AS subject,",
                "    toDate(min(closed_on)) AS metric_date,",
                "    min(closed_on) AS observed_at,",
                "    toFloat64(1) AS contribution,",
                "    arrayStringConcat(arraySort(groupUniqArray(ifNull(toString(title), ''))), ', ') AS display_title,",
                "    arrayStringConcat(arraySort(groupUniqArray(ifNull(toString(pull_request_id), ''))), ', ') AS display_reference,",
                "    arrayStringConcat(arraySort(groupUniqArray(ifNull(toString(author_name), ''))), ', ') AS display_actor,",
                "    arrayStringConcat(arraySort(groupUniqArray(ifNull(toString(repo_slug), ''))), ', ') AS display_location,",
                "    arrayStringConcat(arraySort(groupUniqArray(coalesce(toString(repo_slug), '__unknown__'))), ', ') AS dim_0_value,",
                "    arrayStringConcat(arraySort(groupUniqArray(coalesce(toString(repo_slug), 'Unknown'))), ', ') AS dim_0_label,",
                "    ifNull(toString(author_email), '') AS sort_0,",
                "    ifNull(toString(toDate(closed_on)), '') AS sort_1",
                "FROM silver.class_git_pull_requests FINAL",
                "WHERE tenant_id = ?",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "GROUP BY entity_id, subject",
                "ORDER BY sort_0, sort_1",
                "LIMIT ?",
            ])
        );
    }

    #[test]
    fn a_distinct_count_groups_a_page_by_the_pairs_the_value_counted() {
        let page = only(
            &metric(direct("active_days")),
            &[counting_days()],
            &drilldown_query(),
        );
        let value = compile_metric_query(
            &catalog(),
            &metric(direct("active_days")),
            &measures(&[counting_days()]),
            &query(ViewKind::SubjectTotal),
        )
        .expect("compiles");

        assert!(
            value
                .sql
                .contains("toFloat64(uniqExact(toDate(closed_on))) AS value")
        );
        assert!(page.sql.contains("GROUP BY entity_id, subject"));
        assert!(page.sql.contains("    toDate(closed_on) AS subject,"));
    }

    #[test]
    fn a_ratio_pages_each_half_under_the_part_it_plays() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let closed = measure(
            "prs_closed",
            Some("{ field: state, op: in, value: [merged, closed] }"),
        );

        let compiled = rows(
            &metric(ratio("prs_merged", "prs_closed")),
            &[merged, closed],
            &drilldown_query(),
        );

        assert_eq!(compiled.len(), 2);
        assert_eq!(compiled[0].input_role, ROLE_NUMERATOR);
        assert_eq!(compiled[1].input_role, ROLE_DENOMINATOR);
        assert_eq!(compiled[0].params[0], text("numerator"));
        assert_eq!(compiled[1].params[0], text("denominator"));
        assert!(compiled[0].sql.contains("  AND state = ?"));
        assert!(compiled[1].sql.contains("  AND state IN (?, ?)"));
    }

    #[test]
    fn a_derived_metric_pages_each_input_under_the_alias_it_is_read_by() {
        let merged = measure(
            "prs_merged",
            Some("{ field: state, op: eq, value: merged }"),
        );
        let opened = measure("prs_opened", None);

        let compiled = rows(
            &metric(derived(
                &[("merged", "prs_merged"), ("opened", "prs_opened")],
                "merged / opened",
            )),
            &[merged, opened],
            &drilldown_query(),
        );

        assert_eq!(
            compiled
                .iter()
                .map(|page| page.input_role.as_str())
                .collect::<Vec<_>>(),
            ["merged", "opened"]
        );
        assert_eq!(compiled[0].params[0], text("merged"));
        assert_eq!(compiled[1].params[0], text("opened"));
        assert!(compiled[0].sql.contains("  AND state = ?"));
        assert!(!compiled[1].sql.contains("  AND state"));
    }

    #[test]
    fn a_stddev_pages_the_rows_its_spread_was_measured_over() {
        let compiled = only(
            &metric(stddev("pr_size")),
            &[sized_measure("pr_size")],
            &drilldown_query(),
        );

        assert_eq!(compiled.input_role, ROLE_VALUE);
        assert!(
            compiled
                .sql
                .contains("    toFloat64(lines_added) AS contribution,")
        );
    }

    #[test]
    fn a_percentile_pages_the_rows_its_distribution_ranked() {
        let compiled = only(
            &metric(percentile("pr_size", 0.5)),
            &[sized_measure("pr_size")],
            &drilldown_query(),
        );

        assert_eq!(compiled.input_role, ROLE_VALUE);
        assert!(
            compiled
                .sql
                .contains("    toFloat64(lines_added) AS contribution,")
        );
    }

    #[test]
    fn a_dimension_reports_its_label_field_when_its_value_key_is_not_presentable() {
        let compiled = only(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            &drilldown_query(),
        );

        assert!(compiled.sql.contains(&lines(&[
            "    coalesce(toString(data_source), '__unknown__') AS dim_1_value,",
            "    coalesce(toString(data_source_label), 'Unknown') AS dim_1_label,",
        ])));
        assert_eq!(
            compiled
                .columns
                .iter()
                .find_map(|column| match &column.kind {
                    DrilldownColumnKind::DimensionLabel(key) if key == "source" =>
                        Some(column.alias.as_str()),
                    _ => None,
                }),
            Some("dim_1_label")
        );
    }

    #[test]
    fn a_column_says_what_it_is_rather_than_being_read_back_from_its_name() {
        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &drilldown_query(),
        );

        assert_eq!(
            compiled
                .columns
                .iter()
                .map(|column| column.kind.clone())
                .collect::<Vec<_>>(),
            vec![
                DrilldownColumnKind::EntityId,
                DrilldownColumnKind::InputRole,
                DrilldownColumnKind::Date,
                DrilldownColumnKind::ObservedAt,
                DrilldownColumnKind::Contribution,
                DrilldownColumnKind::Display(DisplayRole::Title),
                DrilldownColumnKind::Display(DisplayRole::Reference),
                DrilldownColumnKind::Display(DisplayRole::Actor),
                DrilldownColumnKind::Display(DisplayRole::Location),
                DrilldownColumnKind::DimensionValue("repository".to_owned()),
                DrilldownColumnKind::DimensionLabel("repository".to_owned()),
                DrilldownColumnKind::SortKey(0),
                DrilldownColumnKind::SortKey(1),
                DrilldownColumnKind::SortKey(2),
            ]
        );
    }

    #[test]
    fn a_display_role_the_dataset_does_not_declare_contributes_no_column() {
        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &drilldown_query(),
        );

        assert!(!aliases(&compiled).contains(&"display_link"));
    }

    #[test]
    fn a_requested_dimension_the_measure_does_not_declare_is_rejected() {
        let mut request = drilldown_query();
        request.display_dimensions = vec!["not_a_dimension".to_owned()];

        assert_eq!(
            rows_err(
                &metric(direct("prs_merged")),
                &[measure("prs_merged", None)],
                &request
            ),
            CompileError::UnknownDimension {
                measure: "prs_merged".to_owned(),
                key: "not_a_dimension".to_owned(),
            }
        );
    }

    #[test]
    fn a_requested_dimension_is_projected_once_however_often_it_is_named() {
        let mut request = drilldown_query();
        request.display_dimensions = vec!["repository".to_owned(), "repository".to_owned()];

        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &request,
        );

        assert_eq!(compiled.sql.matches("AS dim_0_value").count(), 1);
    }

    #[test]
    fn a_people_scoped_page_keys_each_row_by_the_person_its_pool_attributes_it_to() {
        let mut request = drilldown_query();
        request.entity_scope = people();

        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &request,
        );

        assert!(compiled.sql.starts_with(&lines(&pool_head())));
        assert!(compiled.sql.contains("    pool.person_ref AS entity_id,"));
        assert!(
            compiled
                .sql
                .contains("INNER JOIN pool ON pool.identity = author_email")
        );
        assert_eq!(
            compiled.params.get(..people_params().len()),
            Some(people_params().as_slice())
        );
        assert_eq!(compiled.params[people_params().len()], text("value"));
    }

    #[test]
    fn a_page_resumes_from_the_ordering_values_the_previous_page_ended_on() {
        let mut request = drilldown_query();
        request.cursor = Some(DrilldownCursor {
            sort_value: None,
            sort_values: vec!["row-7".to_owned(), "github".to_owned(), "pr-42".to_owned()],
        });

        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &request,
        );

        assert!(
            compiled.sql.contains(
                "  AND tuple(ifNull(toString(unique_key), ''), ifNull(toString(data_source), ''), ifNull(toString(pull_request_id), '')) > tuple(?, ?, ?)"
            ),
            "{}",
            compiled.sql
        );
        assert_eq!(
            compiled.params,
            vec![
                text("value"),
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                text("row-7"),
                text("github"),
                text("pr-42"),
                QueryParam::UInt(51),
            ]
        );
    }

    #[test]
    fn a_subject_page_resumes_on_the_pairs_it_is_ordered_by() {
        let mut request = drilldown_query();
        request.cursor = Some(DrilldownCursor {
            sort_value: None,
            sort_values: vec!["dev@example.com".to_owned(), "2026-01-10".to_owned()],
        });

        let compiled = only(&metric(direct("active_days")), &[counting_days()], &request);

        assert!(
            compiled.sql.contains(
                "  AND tuple(ifNull(toString(author_email), ''), ifNull(toString(toDate(closed_on)), '')) > tuple(?, ?)"
            ),
            "{}",
            compiled.sql
        );
        assert!(compiled.sql.contains("ORDER BY sort_0, sort_1"));
    }

    #[test]
    fn a_position_that_does_not_match_the_ordering_is_refused() {
        let mut request = drilldown_query();
        request.cursor = Some(DrilldownCursor {
            sort_value: None,
            sort_values: vec!["one".to_owned(), "two".to_owned()],
        });

        assert_eq!(
            rows_err(
                &metric(direct("prs_merged")),
                &[measure("prs_merged", None)],
                &request
            ),
            CompileError::CursorArity {
                expected: 3,
                found: 2,
            }
        );
    }

    #[test]
    fn a_page_reads_one_row_beyond_its_size_so_a_further_page_is_detectable() {
        let mut request = drilldown_query();
        request.page_size = 200;

        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &request,
        );

        assert!(compiled.sql.ends_with("LIMIT ?"));
        assert_eq!(compiled.params.last(), Some(&QueryParam::UInt(201)));
    }

    #[test]
    fn every_placeholder_has_exactly_one_bound_parameter() {
        let mut request = drilldown_query();
        request.entity_scope = people();
        request.dimension_filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: vec!["example/app".to_owned()],
        }];
        request.cursor = Some(DrilldownCursor {
            sort_value: None,
            sort_values: vec!["dev@example.com".to_owned(), "2026-01-10".to_owned()],
        });

        let compiled = only(&metric(direct("active_days")), &[counting_days()], &request);

        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn a_page_orders_by_the_row_identity_the_dataset_declares_as_well_as_its_sorting_key() {
        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &drilldown_query(),
        );

        let sort_keys: Vec<&str> = compiled
            .columns
            .iter()
            .filter(|column| matches!(column.kind, DrilldownColumnKind::SortKey(_)))
            .map(|column| column.alias.as_str())
            .collect();

        assert_eq!(sort_keys, ["sort_0", "sort_1", "sort_2"]);
        assert!(compiled.sql.contains(&lines(&[
            "    ifNull(toString(unique_key), '') AS sort_0,",
            "    ifNull(toString(data_source), '') AS sort_1,",
            "    ifNull(toString(pull_request_id), '') AS sort_2",
        ])));
    }

    #[test]
    fn a_column_that_both_orders_and_identifies_a_row_is_ordered_by_once() {
        let mut catalog = catalog();
        let dataset = catalog
            .datasets
            .iter_mut()
            .find(|dataset| dataset.key == "git_pull_requests")
            .expect("dataset is catalogued");
        dataset.sorting_key = vec!["data_source".to_owned(), "unique_key".to_owned()];
        dataset.row_identity = vec!["unique_key".to_owned(), "pull_request_id".to_owned()];

        let compiled = compile_drilldown(
            &catalog,
            &metric(direct("prs_merged")),
            &measures(&[measure("prs_merged", None)]),
            &drilldown_query(),
        )
        .expect("compiles");

        assert_eq!(
            compiled[0]
                .sql
                .matches("ifNull(toString(unique_key), '') AS sort_")
                .count(),
            1
        );
        assert!(compiled[0].sql.contains("ORDER BY sort_0, sort_1, sort_2"));
    }

    #[test]
    fn a_dataset_that_declares_no_row_identity_is_refused_a_page_of_its_rows() {
        let mut catalog = catalog();
        let dataset = catalog
            .datasets
            .iter_mut()
            .find(|dataset| dataset.key == "git_pull_requests")
            .expect("dataset is catalogued");
        dataset.row_identity = Vec::new();

        assert_eq!(
            compile_drilldown(
                &catalog,
                &metric(direct("prs_merged")),
                &measures(&[measure("prs_merged", None)]),
                &drilldown_query()
            )
            .expect_err("expected a compile error"),
            CompileError::UnorderableDataset {
                dataset: "git_pull_requests".to_owned(),
                reason: "it declares no row identity, so no order over its rows is provably total"
                    .to_owned(),
            }
        );
    }

    #[test]
    fn a_dataset_whose_row_order_is_not_readable_from_its_columns_is_refused() {
        let mut catalog = catalog();
        let dataset = catalog
            .datasets
            .iter_mut()
            .find(|dataset| dataset.key == "git_pull_requests")
            .expect("dataset is catalogued");
        dataset.sorting_key = vec!["cityHash64(unique_key)".to_owned()];

        assert_eq!(
            compile_drilldown(
                &catalog,
                &metric(direct("prs_merged")),
                &measures(&[measure("prs_merged", None)]),
                &drilldown_query()
            )
            .expect_err("expected a compile error"),
            CompileError::UnorderableDataset {
                dataset: "git_pull_requests".to_owned(),
                reason: "it orders by `cityHash64(unique_key)`, which is not one of its columns"
                    .to_owned(),
            }
        );
    }

    #[test]
    fn a_dimension_column_is_named_by_its_position_rather_than_by_its_key() {
        let mut named = measure("prs_merged", None);
        named.dimensions[0].key = "repo slug\"; --".to_owned();

        let compiled = only(&metric(direct("prs_merged")), &[named], &drilldown_query());

        assert!(!compiled.sql.contains("repo slug"), "{}", compiled.sql);
        assert!(compiled.sql.contains("AS dim_0_value,"));
        assert_eq!(
            compiled
                .columns
                .iter()
                .find_map(|column| match &column.kind {
                    DrilldownColumnKind::DimensionValue(key) => Some(key.as_str()),
                    _ => None,
                }),
            Some("repo slug\"; --"),
            "the key still reaches the reader, off the statement"
        );
    }

    #[test]
    fn filter_values_never_reach_the_sql_text() {
        let injection = "'; DROP TABLE x; --";
        let mut request = drilldown_query();
        request.entity_scope = EntityScope::Identities(vec![injection.to_owned()]);
        request.cursor = Some(DrilldownCursor {
            sort_value: None,
            sort_values: vec![injection.to_owned(); 3],
        });

        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure(
                "prs_merged",
                Some("{ field: state, op: eq, value: merged }"),
            )],
            &request,
        );

        assert!(!compiled.sql.contains("DROP TABLE"), "{}", compiled.sql);
        assert_eq!(
            compiled
                .params
                .iter()
                .filter(|param| **param == text(injection))
                .count(),
            4
        );
    }

    #[test]
    fn a_page_of_a_window_the_request_did_not_carry_a_measure_for_is_rejected() {
        assert_eq!(
            rows_err(
                &metric(direct("prs_merged")),
                &[sized_measure("pr_size")],
                &drilldown_query()
            ),
            CompileError::MeasureNotFound {
                metric: "git.merge_rate".to_owned(),
                measure: "prs_merged".to_owned(),
            }
        );
    }

    #[test]
    fn the_inputs_a_metric_is_paged_by_are_the_parts_its_pages_are_tagged_with() {
        let cases: [(&str, MetricDefinition, Vec<MeasureDefinition>); 3] = [
            (
                "direct",
                metric(direct("prs_merged")),
                vec![measure("prs_merged", None)],
            ),
            (
                "ratio",
                metric(ratio("prs_merged", "prs_closed")),
                vec![measure("prs_merged", None), measure("prs_closed", None)],
            ),
            (
                "derived",
                metric(derived(
                    &[("merged", "prs_merged"), ("opened", "prs_opened")],
                    "merged / opened",
                )),
                vec![measure("prs_merged", None), measure("prs_opened", None)],
            ),
        ];

        for (named, metric, defined) in cases {
            let roles = drilldown_input_roles(&metric, &measures(&defined)).expect("resolves");

            assert_eq!(
                roles,
                rows(&metric, &defined, &drilldown_query())
                    .iter()
                    .map(|page| page.input_role.clone())
                    .collect::<Vec<_>>(),
                "{named}"
            );
        }
    }

    #[test]
    fn a_page_says_which_relation_it_scanned_and_whether_its_contribution_says_anything() {
        let counted = only(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &drilldown_query(),
        );
        let measured = only(
            &metric(direct("pr_size")),
            &[sized_measure("pr_size")],
            &drilldown_query(),
        );
        let distinct = only(
            &metric(direct("active_days")),
            &[counting_days()],
            &drilldown_query(),
        );

        assert_eq!(counted.relation.database, "silver");
        assert_eq!(counted.relation.relation, "class_git_pull_requests");
        assert_eq!(counted.contribution, Contribution::CountedRow);
        assert_eq!(distinct.contribution, Contribution::CountedRow);
        assert_eq!(measured.contribution, Contribution::MeasuredValue);
    }

    #[test]
    fn a_page_reads_the_window_the_request_names() {
        let mut request = drilldown_query();
        request.from = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        request.to = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();

        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &request,
        );

        assert!(compiled.params.contains(&text("2026-02-01")));
        assert!(compiled.params.contains(&text("2026-02-28")));
    }

    fn sorted(column: &str, direction: SortDirection) -> DrilldownQuery {
        DrilldownQuery {
            sort: Some(DrilldownSort {
                column: column.to_owned(),
                direction,
            }),
            ..drilldown_query()
        }
    }

    fn order_by(sql: &str) -> &str {
        sql.lines()
            .find(|line| line.starts_with("ORDER BY "))
            .expect("a page is ordered")
    }

    /// The keyset compare is the only tuple in a page, wherever the clause it
    /// sits in.
    fn resume_clause(sql: &str) -> &str {
        sql.lines()
            .find(|line| line.contains("tuple("))
            .unwrap_or_else(|| panic!("a resumed page carries a resume predicate: {sql}"))
    }

    #[test]
    fn a_page_sorted_ascending_orders_by_that_column_before_the_order_it_falls_back_on() {
        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &sorted("date", SortDirection::Ascending),
        );

        assert_eq!(
            order_by(&compiled.sql),
            "ORDER BY metric_date ASC NULLS LAST, sort_0, sort_1, sort_2"
        );
    }

    #[test]
    fn a_page_sorted_descending_reports_the_rows_carrying_no_value_last_all_the_same() {
        let compiled = only(
            &metric(direct("pr_size")),
            &[sized_measure("pr_size")],
            &sorted("value", SortDirection::Descending),
        );

        assert_eq!(
            order_by(&compiled.sql),
            "ORDER BY contribution DESC NULLS LAST, sort_0, sort_1, sort_2"
        );
    }

    /// The expansion is what a tuple compare cannot say: which side of the
    /// sorted value the unread rows are on, and that the nulls are all still
    /// ahead of the page.
    #[test]
    fn a_sorted_page_resumes_after_the_value_it_ended_on_and_before_the_nulls_it_has_not_reached() {
        let mut request = sorted("value", SortDirection::Descending);
        request.cursor = Some(DrilldownCursor {
            sort_value: Some(SortValue::Number(12.0)),
            sort_values: vec!["row-7".to_owned(), "github".to_owned(), "pr-42".to_owned()],
        });

        let compiled = only(
            &metric(direct("pr_size")),
            &[sized_measure("pr_size")],
            &request,
        );

        assert_eq!(
            resume_clause(&compiled.sql),
            "  AND (toFloat64(lines_added) < ? OR toFloat64(lines_added) IS NULL OR \
             (toFloat64(lines_added) = ? AND tuple(ifNull(toString(unique_key), ''), \
             ifNull(toString(data_source), ''), ifNull(toString(pull_request_id), '')) > \
             tuple(?, ?, ?)))"
        );
        assert_eq!(
            compiled.params,
            vec![
                text("value"),
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                QueryParam::Float(12.0),
                QueryParam::Float(12.0),
                text("row-7"),
                text("github"),
                text("pr-42"),
                QueryParam::UInt(51),
            ]
        );
    }

    /// Once a page is inside the nulls the sort reports last, only the total
    /// order separates the rows, so the sorted column stops narrowing.
    #[test]
    fn a_page_resuming_inside_the_null_tail_is_carried_by_the_total_order_alone() {
        let mut request = sorted("value", SortDirection::Ascending);
        request.cursor = Some(DrilldownCursor {
            sort_value: None,
            sort_values: vec!["row-7".to_owned(), "github".to_owned(), "pr-42".to_owned()],
        });

        let compiled = only(
            &metric(direct("pr_size")),
            &[sized_measure("pr_size")],
            &request,
        );

        assert_eq!(
            resume_clause(&compiled.sql),
            "  AND (toFloat64(lines_added) IS NULL AND \
             tuple(ifNull(toString(unique_key), ''), ifNull(toString(data_source), ''), \
             ifNull(toString(pull_request_id), '')) > tuple(?, ?, ?))"
        );
        assert_eq!(
            compiled.params,
            vec![
                text("value"),
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                text("row-7"),
                text("github"),
                text("pr-42"),
                QueryParam::UInt(51),
            ]
        );
    }

    /// A subject row's columns are folded from the events it collapsed, so the
    /// scan cannot filter on one: the resume waits until the grouping has run.
    #[test]
    fn a_sorted_subject_page_resumes_after_its_rows_are_folded_rather_than_on_the_scan() {
        let mut request = sorted("title", SortDirection::Ascending);
        request.cursor = Some(DrilldownCursor {
            sort_value: Some(SortValue::Text("a title".to_owned())),
            sort_values: vec!["dev@example.com".to_owned(), "2026-01-10".to_owned()],
        });

        let compiled = only(&metric(direct("active_days")), &[counting_days()], &request);

        let folded =
            "arrayStringConcat(arraySort(groupUniqArray(ifNull(toString(title), ''))), ', ')";
        assert_eq!(
            resume_clause(&compiled.sql),
            format!(
                "HAVING ({folded} > ? OR {folded} IS NULL OR ({folded} = ? AND \
                 tuple(ifNull(toString(author_email), ''), \
                 ifNull(toString(toDate(closed_on)), '')) > tuple(?, ?)))"
            )
        );
        assert!(
            !where_block(&compiled.sql).contains("groupUniqArray"),
            "a folded column cannot be filtered on the scan: {}",
            compiled.sql
        );
        assert_eq!(
            order_by(&compiled.sql),
            "ORDER BY display_title ASC NULLS LAST, sort_0, sort_1"
        );
    }

    /// A page naming no column carries nothing of the sorted form: its order is
    /// the total order alone, and its resume the one compare that expresses it.
    #[test]
    fn a_page_that_names_no_column_is_ordered_and_resumed_by_its_total_order_alone() {
        let mut request = drilldown_query();
        request.cursor = Some(DrilldownCursor {
            sort_value: None,
            sort_values: vec!["row-7".to_owned(), "github".to_owned(), "pr-42".to_owned()],
        });

        let compiled = only(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &request,
        );

        assert_eq!(order_by(&compiled.sql), "ORDER BY sort_0, sort_1, sort_2");
        assert_eq!(
            resume_clause(&compiled.sql),
            "  AND tuple(ifNull(toString(unique_key), ''), ifNull(toString(data_source), ''), \
             ifNull(toString(pull_request_id), '')) > tuple(?, ?, ?)"
        );
        assert!(!compiled.sql.contains("NULLS LAST"));
        assert!(!compiled.sql.contains("HAVING"));
    }

    #[test]
    fn a_column_the_page_does_not_report_cannot_be_ordered_by_and_the_refusal_names_what_can() {
        assert_eq!(
            rows_err(
                &metric(direct("prs_merged")),
                &[measure("prs_merged", None)],
                &sorted("not_a_column", SortDirection::Ascending)
            ),
            CompileError::UnsortableColumn {
                column: "not_a_column".to_owned(),
                sortable: "`subject`, `date`, `observed_at`, `title`, `reference`, `actor`, \
                           `location`, `repository_value`, `repository`"
                    .to_owned(),
            }
        );
    }

    /// A counted row contributes the same 1 as every other, so the page reports
    /// no number of its own and there is nothing there to order by.
    #[test]
    fn a_column_the_page_projects_but_does_not_report_cannot_be_ordered_by_either() {
        let refused = rows_err(
            &metric(direct("prs_merged")),
            &[measure("prs_merged", None)],
            &sorted("value", SortDirection::Ascending),
        );

        assert!(
            matches!(refused, CompileError::UnsortableColumn { ref column, .. } if column == "value"),
            "{refused}"
        );
        assert!(
            only(
                &metric(direct("pr_size")),
                &[sized_measure("pr_size")],
                &sorted("value", SortDirection::Ascending)
            )
            .sql
            .contains("ORDER BY contribution ASC NULLS LAST"),
            "a measured page does report the value it folded"
        );
    }

    #[test]
    fn what_a_page_may_be_ordered_by_is_exactly_what_it_reports() {
        let reported = drilldown_reported_columns(
            &catalog(),
            &metric(direct("prs_merged")),
            &measures(&[measure("prs_merged", None)]),
            &[],
        )
        .expect("resolves");

        assert_eq!(
            reported.get(ROLE_VALUE).map(Vec::as_slice),
            Some(
                [
                    "subject",
                    "date",
                    "observed_at",
                    "title",
                    "reference",
                    "actor",
                    "location",
                    "repository_value",
                    "repository",
                ]
                .map(str::to_owned)
                .as_slice()
            )
        );
    }

    #[test]
    fn every_placeholder_of_a_sorted_page_has_exactly_one_bound_parameter() {
        let mut request = sorted("value", SortDirection::Descending);
        request.entity_scope = people();
        request.dimension_filters = vec![DimensionFilter {
            key: "repository".to_owned(),
            values: vec!["example/app".to_owned()],
        }];
        request.cursor = Some(DrilldownCursor {
            sort_value: Some(SortValue::Number(3.5)),
            sort_values: vec!["row-7".to_owned(), "github".to_owned(), "pr-42".to_owned()],
        });

        let compiled = only(
            &metric(direct("pr_size")),
            &[sized_measure("pr_size")],
            &request,
        );

        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }
}
