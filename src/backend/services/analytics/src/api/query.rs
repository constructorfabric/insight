//! `POST /v1/query` — one query contract over the declared datasets.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::AppState;
use super::error::QueryError;
use crate::domain::query::compile::params::QueryParam;
use crate::domain::query::compile::{self, CompiledQuery};
use crate::domain::query::contract::dto::{QueryAnswer, QueryRequest};
use crate::domain::query::violation::Violation;
use crate::domain::query::{answer, validation};
use crate::infra::metrics::QueryKind;
use crate::infra::query::fetch_bound_rows;

pub async fn query(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(request): Json<QueryRequest>,
) -> Result<Json<QueryAnswer>, CanonicalError> {
    let plan = validation::plan(&request).map_err(|violations| refused(&violations))?;

    // INVARIANT: the scan's tenancy binds from the session, never from the query.
    let tenant_id = ctx.subject_tenant_id().to_string();
    let CompiledQuery { sql, params } = compile::compile(&plan, &tenant_id);
    let bindings: Vec<serde_json::Value> = params.iter().map(QueryParam::binding).collect();
    let comment = format!("query:{}", plan.dataset.key);

    let returned = fetch_bound_rows(&state.ch, &sql, &bindings, QueryKind::Query, &comment)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, comment, "query failed");
            CanonicalError::internal("failed to answer the query").create()
        })?;

    let answer = answer::assemble(&plan, returned).map_err(|error| {
        tracing::error!(error = %error, comment, "query answer assembly failed");
        CanonicalError::internal("failed to answer the query").create()
    })?;

    Ok(Json(answer))
}

// INVARIANT: the refusal path is reached only with at least one violation, which
// the builder needs to open a field-violation envelope.
fn refused(violations: &[Violation]) -> CanonicalError {
    let Some((first, rest)) = violations.split_first() else {
        return CanonicalError::internal("a query was refused without a reason").create();
    };

    let mut refusal = QueryError::invalid_argument().with_field_violation(
        &first.field,
        &first.detail,
        first.reason.as_code(),
    );
    for violation in rest {
        refusal = refusal.with_field_violation(
            &violation.field,
            &violation.detail,
            violation.reason.as_code(),
        );
    }

    refusal.create()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use toolkit_canonical_errors::Problem;

    use super::*;
    use crate::domain::query::violation::Reason;

    #[test]
    fn a_refusal_reports_every_violation_with_its_field_and_machine_code() {
        let problem = Problem::from(refused(&[
            Violation::new(
                "limit",
                Reason::OutOfRange,
                "an answer carries at most 10000 rows",
            ),
            Violation::unknown("group_by[0].field", "branch", &["repository"]),
        ]));
        let problem = serde_json::to_value(problem).expect("the envelope serializes");

        assert_eq!(problem["status"], 400);
        assert_eq!(
            problem["context"]["resource_type"],
            "gts.cf.insight.analytics_api.query.v1~"
        );
        let violations = problem["context"]["field_violations"]
            .as_array()
            .expect("field_violations is an array");
        assert_eq!(violations.len(), 2);
        assert_eq!(violations[0]["field"], "limit");
        assert_eq!(violations[0]["reason"], "OUT_OF_RANGE");
        assert_eq!(violations[1]["field"], "group_by[0].field");
        assert_eq!(violations[1]["reason"], "UNKNOWN");
    }
}
