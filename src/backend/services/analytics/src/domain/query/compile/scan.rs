//! What a scan reads and what it is scoped to.
//!
//! INVARIANT: tenancy leads every read, bound from the session and never from
//! the request.

use crate::domain::field_catalog::model::ReadDiscipline;
use crate::domain::query::plan::QueryPlan;

use super::filters;
use super::params::QueryParam;
use super::time::event_day;

pub fn from_clause(plan: &QueryPlan<'_>) -> String {
    let relation = format!("{}.{}", plan.dataset.database, plan.dataset.relation);
    match plan.dataset.read_discipline {
        ReadDiscipline::Plain => relation,
        ReadDiscipline::Final => format!("{relation} FINAL"),
    }
}

/// The `WHERE` predicates, in binding order: tenancy, window, then row filters.
pub fn predicates(
    plan: &QueryPlan<'_>,
    tenant_id: &str,
    params: &mut Vec<QueryParam>,
) -> Vec<String> {
    let mut predicates = vec![format!("{} = ?", plan.dataset.tenant_field)];
    params.push(QueryParam::Text(tenant_id.to_owned()));

    let day = event_day(plan.time.field);
    predicates.push(format!("{day} >= toDate(?)"));
    params.push(QueryParam::Text(plan.time.from.to_string()));
    predicates.push(format!("{day} <= toDate(?)"));
    params.push(QueryParam::Text(plan.time.to.to_string()));

    for filter in &plan.filters {
        predicates.push(filters::render(filter, params));
    }

    predicates
}
