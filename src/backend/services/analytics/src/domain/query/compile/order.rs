//! How the answer's rows are ordered and how many of them there are.
//!
//! INVARIANT: a grouped query orders by every group column the query did not
//! name, so a row ceiling truncates the same rows on every run.

use crate::domain::query::contract::dto::Direction;
use crate::domain::query::plan::{QueryPlan, column_alias};

pub fn terms(plan: &QueryPlan<'_>) -> Vec<String> {
    let mut ordered: Vec<usize> = Vec::with_capacity(plan.order.len() + plan.group_by.len());
    let mut terms = Vec::with_capacity(ordered.capacity());

    for term in &plan.order {
        ordered.push(term.column);
        terms.push(format!(
            "{} {}",
            column_alias(term.column),
            direction(term.direction)
        ));
    }

    for column in 0..plan.group_by.len() {
        if !ordered.contains(&column) {
            terms.push(format!("{} ASC", column_alias(column)));
        }
    }

    terms
}

fn direction(direction: Direction) -> &'static str {
    match direction {
        Direction::Asc => "ASC",
        Direction::Desc => "DESC",
    }
}
