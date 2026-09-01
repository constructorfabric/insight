use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
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
use super::listing::{self, CursorRejected, PagePosition};
use super::resolution::PersonSummaryResponse;
use crate::domain::person_card;
use crate::infra::db::person_listing::{self, PersonListRow, VisibleTo};
use crate::infra::db::{persons_repo, subchart_repo};

// One bound parameter per person id, so the request bounds the query. Equal to
// the analytics metric-results cap on purpose: that endpoint forwards a whole
// cleared request here, so a smaller cap would reject a request analytics
// already accepted.
pub(super) const MAX_PERSON_IDS: usize = 1000;

/// Browsed rather than narrowed, so the page is larger than the picker's.
const ROSTER_DEFAULT_LIMIT: u64 = 50;
const ROSTER_MAX_LIMIT: u64 = 500;

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

#[derive(Debug, Deserialize)]
pub struct RosterParams {
    pub q: Option<String>,
    // Signed so a nonsense `?limit=` clamps rather than failing deserialization,
    // matching the other listings.
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// Where the next roster page resumes. Opaque on the wire; the shape is ours.
#[derive(Debug, Serialize, Deserialize)]
struct PageKey {
    order_key: String,
    person_id: Uuid,
}

impl PagePosition for PageKey {
    // INVARIANT: distinct from the picker's — a position ordered over one set
    // must not resume a listing that ordered another.
    const KIND: &'static str = "visible-persons";
}

/// One page of the persons the caller may see.
#[derive(Debug, Serialize, ToSchema)]
pub struct VisiblePersonsPageResponse {
    pub items: Vec<PersonSummaryResponse>,
    /// Pass back as `?cursor=` for the next page; absent on the last one.
    pub next_cursor: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for VisiblePersonsPageResponse {}

/// Self-scoped and not admin-gated, like the POST: it enumerates only what the
/// caller's visible set already contains.
///
/// Lists the ROSTER, not the journal: only persons a connector claims as an
/// account holder. So this answers for a narrower set than the POST confirms —
/// deliberately, and in the safe direction (a person listed here is a person
/// that filter would confirm, never the reverse).
pub async fn list_visible_persons(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(params): Query<RosterParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    let caller = require_caller(&ctx)?;
    let tenant = ctx.subject_tenant_id();

    let terms = listing::search_terms(params.q.as_deref().unwrap_or_default())
        .map_err(|message| invalid_query("q", &message))?;
    let limit = listing::clamp_limit(params.limit, ROSTER_DEFAULT_LIMIT, ROSTER_MAX_LIMIT);

    let query = terms.join(" ");
    let resume = listing::resume_from::<PageKey>(params.cursor.as_deref(), tenant, &query)
        .map_err(|rejected: CursorRejected| invalid_query("cursor", rejected.message()))?;

    let rows = person_listing::list_persons(
        &state.db,
        tenant,
        &terms,
        &[],
        // The roster is who the organisation IS. An address a commit carried
        // becomes a person in the journal without anyone here holding it, and
        // listing those alongside the members reads as a directory of strangers.
        person_listing::Restrict {
            listed: person_listing::Listed::AccountHolders,
            visible_to: Some(VisibleTo {
                viewer_person_id: caller,
                org_source_type: &state.config.org_chart_source_type,
                policy: state.config.visibility_policy,
            }),
        },
        resume.as_ref().map(|key| person_listing::After {
            order_key: &key.order_key,
            person_id: key.person_id,
        }),
        limit + 1,
    )
    .await
    .map_err(read_err)?;

    let (rows, next_cursor) =
        listing::cut_to_page(rows, limit, tenant, &query, |row: &PersonListRow| PageKey {
            order_key: row.order_key.clone(),
            person_id: row.person_id,
        })
        .map_err(|e| {
            tracing::error!(error = %e, "failed to issue a roster page cursor");
            CanonicalError::internal("failed to list visible persons").create()
        })?;

    let ids: Vec<Uuid> = rows.iter().map(|row| row.person_id).collect();
    let cards = persons_repo::person_cards(&state.db, tenant, &ids)
        .await
        .map_err(read_err)?;
    let mut items: Vec<PersonSummaryResponse> = person_card::in_requested_order(&ids, &cards)
        .into_iter()
        .map(PersonSummaryResponse::from)
        .collect();
    super::resolution::mark_provisional(&state, tenant, &mut items).await?;

    Ok(Json(VisiblePersonsPageResponse { items, next_cursor }))
}

fn invalid_query(field: &str, detail: &str) -> CanonicalError {
    VisibilityError::invalid_argument()
        .with_field_violation(field, detail, "invalid_query")
        .create()
}

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

fn read_err(e: impl std::fmt::Display) -> CanonicalError {
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
