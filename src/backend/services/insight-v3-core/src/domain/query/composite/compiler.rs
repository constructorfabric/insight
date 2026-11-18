//! Compiling a composite: its children's rows stacked into one relation, then
//! folded to one row per key.

use std::collections::HashMap;

use super::{Alias, Composite, Field};
use crate::domain::kinds::dataset::declaration::BUCKET_COLUMN;
use crate::domain::query::metric_query::{
    Bound, CompiledQuery, FieldType, FilterBind, MetricQuery, bound_by, ratio_sql,
};

/// One combined metric, compiled as a subquery for the composite to stack.
#[derive(Debug)]
pub(crate) struct Stacked<'a> {
    pub(crate) alias: &'a Alias,
    pub(crate) query: &'a MetricQuery,
    pub(crate) compiled: CompiledQuery,
}

/// The fold's select list, and what a reader is told about its columns.
#[derive(Debug, Default)]
struct Folded {
    parts: Vec<String>,
    column_types: HashMap<String, FieldType>,
    percents: Vec<String>,
    exacts: Vec<String>,
}

impl Folded {
    /// A picked column reads as it did in the child it came from.
    fn carry(&mut self, child: &Stacked<'_>, column: &str, as_name: &str) {
        if let Some(field_type) = child.compiled.column_type(column) {
            self.column_types.insert(as_name.to_owned(), field_type);
        }
        if child.compiled.percents().iter().any(|name| name == column) {
            self.percents.push(as_name.to_owned());
        }
        if child.compiled.exacts().iter().any(|name| name == column) {
            self.exacts.push(as_name.to_owned());
        }
    }
}

impl Composite {
    /// The composite over its children, given in the order it combines them.
    ///
    /// INVARIANT: stacked and folded, never joined. A join fills a side with
    /// no row with its type's default unless `join_use_nulls` is set, and
    /// leaves two copies of each key to merge; the fold has neither problem.
    pub(crate) fn compile(&self, children: &[Stacked<'_>], bucketed: bool) -> CompiledQuery {
        let mut keys: Vec<String> = Vec::with_capacity(self.group_by.len() + 1);
        if bucketed {
            keys.push(BUCKET_COLUMN.to_owned());
        }
        keys.extend(self.group_by.iter().cloned());
        let backticked: Vec<String> = keys.iter().map(|key| format!("`{key}`")).collect();

        let (arms, binds) = self.stacked(children, &backticked);
        let folded = self.folded(children);

        let mut selected = backticked.clone();
        selected.extend(folded.parts);
        let mut sql = format!(
            "SELECT {} FROM ({})",
            selected.join(", "),
            arms.join(" UNION ALL ")
        );
        if !backticked.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(&backticked.join(", "));
        }

        // INVARIANT: a run that does not window has no bucket to order by,
        // so the keys order it then.
        let ordering = self
            .order_by
            .as_ref()
            .filter(|order| bucketed || order.field != BUCKET_COLUMN);
        bound_by(&mut sql, Bound::Whole, self.limit, ordering, &keys, &[]);

        CompiledQuery::composed(
            sql,
            binds,
            folded.column_types,
            folded.percents,
            folded.exacts,
        )
    }

    /// One arm per child: its keys, the values it is picked for each in a
    /// slot of their own, and `NULL` in every other slot.
    fn stacked(&self, children: &[Stacked<'_>], keys: &[String]) -> (Vec<String>, Vec<FilterBind>) {
        let mut arms = Vec::with_capacity(children.len());
        let mut binds = Vec::new();

        for child in children {
            let mut columns = keys.to_vec();
            for (index, field) in self.fields.iter().enumerate() {
                let Field::Pick { alias, column, .. } = field else {
                    continue;
                };
                let value = if alias == child.alias.as_str() {
                    format!("`{column}`")
                } else {
                    "NULL".to_owned()
                };
                columns.push(format!("{value} AS `{}`", slot(index)));
            }

            arms.push(format!(
                "SELECT {} FROM ({})",
                columns.join(", "),
                child.compiled.sql()
            ));
            // INVARIANT: binds are positional, so they follow the order the
            // children's SQL appears in.
            binds.extend_from_slice(child.compiled.binds());
        }

        (arms, binds)
    }

    /// Each field over the stacked rows of one key: a pick is the one value
    /// its child holds there, a ratio divides two earlier fields.
    fn folded(&self, children: &[Stacked<'_>]) -> Folded {
        let mut folded = Folded::default();
        if let Some(first) = children.first() {
            for key in &self.group_by {
                if let Some(field_type) = first.compiled.column_type(key) {
                    folded.column_types.insert(key.clone(), field_type);
                }
            }
        }

        for (index, field) in self.fields.iter().enumerate() {
            let expression = match field {
                Field::Pick {
                    as_name,
                    alias,
                    column,
                } => {
                    let read = children.iter().find(|child| child.alias.as_str() == alias);
                    if let Some(child) = read {
                        folded.carry(child, column, as_name);
                    }

                    picked(index, read.is_some_and(|child| child.query.adds_up(column)))
                }
                Field::Ratio {
                    as_name,
                    numerator,
                    denominator,
                    percent,
                } => {
                    folded
                        .column_types
                        .insert(as_name.clone(), FieldType::Float);
                    if *percent {
                        folded.percents.push(as_name.clone());
                    }

                    ratio_sql(numerator, denominator, *percent)
                }
            };
            folded
                .parts
                .push(format!("{expression} AS `{}`", field.as_name()));
        }

        folded
    }
}

/// A slot folded to its key's one value. A key a child has no row for counts
/// nothing there, and a count or a sum of nothing is 0.
fn picked(index: usize, adds_up: bool) -> String {
    let one = format!("any(`{}`)", slot(index));

    if adds_up {
        format!("coalesce({one}, 0)")
    } else {
        one
    }
}

/// The column a field's values are stacked under.
fn slot(index: usize) -> String {
    format!("__c{index}")
}
