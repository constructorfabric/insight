use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::canonical_json::CanonicalJson;
use super::error::VisibilityError;
use super::gate::require_caller;
use crate::infra::db::{persons_repo, subchart_repo};

// One bound parameter per person id, so the request bounds the query. Equal to
// the analytics metric-results cap on purpose: that endpoint forwards a whole
// cleared request here, so a smaller cap would reject a request analytics
// already accepted.
pub(super) const MAX_PERSON_IDS: usize = 1000;

/// Canonical person UUIDs to check (the metric runtime's key since the
/// identity cutover — the earlier email-based draft of this endpoint never
/// shipped).
#[derive(Debug, Deserialize, ToSchema)]
pub struct VisiblePersonsRequest {
    pub person_ids: Vec<Uuid>,
}
impl toolkit::api::api_dto::RequestApiDto for VisiblePersonsRequest {}

#[derive(Debug, Serialize, ToSchema)]
pub struct VisiblePersonsResponse {
    pub visible: Vec<Uuid>,
}
impl toolkit::api::api_dto::ResponseApiDto for VisiblePersonsResponse {}

pub async fn filter_visible_persons(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<VisiblePersonsRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let caller = require_caller(&ctx)?;
    let tenant = ctx.subject_tenant_id();

    let requested = dedup_person_ids(&req.person_ids)?;

    // A wildcard grant covers everyone IN THE TENANT, not everyone whose UUID
    // the caller can type: the echo is intersected with the tenant's persons
    // log, or a wildcard holder in tenant A could get tenant B's ids confirmed
    // as visible — and analytics treats this answer as authorization. Not an
    // existence oracle beyond what the grant already implies: a wildcard
    // holder may see every person the tenant has.
    let policy = state.config.visibility_policy;
    let whole_tenant = policy.is_flat()
        || subchart_repo::has_wildcard_grant(&state.db, tenant, caller)
            .await
            .map_err(read_err)?;

    let visible = if whole_tenant {
        persons_repo::persons_in_tenant(&state.db, tenant, &requested)
            .await
            .map_err(read_err)?
    } else {
        let visible = subchart_repo::visible_targets(
            &state.db,
            tenant,
            caller,
            &requested,
            &state.config.org_chart_source_type,
            policy,
        )
        .await
        .map_err(read_err)?
        .into_iter()
        .collect::<HashSet<_>>();

        requested
            .into_iter()
            .filter(|person_id| visible.contains(person_id))
            .collect()
    };

    Ok(Json(VisiblePersonsResponse { visible }))
}

fn dedup_person_ids(person_ids: &[Uuid]) -> Result<Vec<Uuid>, CanonicalError> {
    if person_ids.len() > MAX_PERSON_IDS {
        return Err(invalid(&format!(
            "at most {MAX_PERSON_IDS} person ids per request"
        )));
    }

    let mut seen: HashSet<Uuid> = HashSet::with_capacity(person_ids.len());
    let mut out: Vec<Uuid> = Vec::with_capacity(person_ids.len());
    for person_id in person_ids {
        if person_id.is_nil() {
            continue;
        }
        if seen.insert(*person_id) {
            out.push(*person_id);
        }
    }

    if out.is_empty() {
        return Err(invalid("person_ids must not be empty"));
    }

    Ok(out)
}

fn invalid(detail: &str) -> CanonicalError {
    VisibilityError::invalid_argument()
        .with_field_violation("person_ids", detail, "invalid_person_ids")
        .create()
}

#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn read_err(e: anyhow::Error) -> CanonicalError {
    tracing::error!(error = %e, "visibility check failed");
    CanonicalError::internal("failed to evaluate visibility").create()
}

#[cfg(test)]
#[expect(clippy::expect_used, reason = "test setup panics on a broken fixture")]
mod tests {
    use super::*;

    #[test]
    fn nil_and_duplicate_person_ids_collapse() {
        let got = dedup_person_ids(&[
            Uuid::from_u128(1),
            Uuid::from_u128(1),
            Uuid::nil(),
            Uuid::from_u128(2),
        ])
        .expect("a non-empty list");

        assert_eq!(got, vec![Uuid::from_u128(1), Uuid::from_u128(2)]);
    }

    #[test]
    fn an_all_nil_list_is_rejected() {
        assert!(dedup_person_ids(&[Uuid::nil()]).is_err());
        assert!(dedup_person_ids(&[]).is_err());
    }

    #[test]
    fn more_person_ids_than_the_cap_are_rejected() {
        let many = (0..=MAX_PERSON_IDS)
            .map(|i| Uuid::from_u128(i as u128 + 1))
            .collect::<Vec<_>>();
        assert!(dedup_person_ids(&many).is_err(), "over-cap rejected");

        let at_cap = (0..MAX_PERSON_IDS)
            .map(|i| Uuid::from_u128(i as u128 + 1))
            .collect::<Vec<_>>();
        assert!(
            dedup_person_ids(&at_cap).is_ok(),
            "the cap itself is allowed"
        );
    }
}
