//! Person listing — the operator's roster and the picker over it.
//!
//! `GET /v1/persons` lists every person of the caller's tenant, ordered by the
//! name the row shows, a page at a time. `?q=` narrows the same list: every
//! whitespace-separated term must match some CURRENT observed value (the
//! latest observation per person × source × value type — the same window rule
//! `/v1/profiles` resolves by, so search and resolution cannot disagree).
//! Superseded values stop matching their old owner; a value two persons both
//! currently claim returns both — the operator is the disambiguator, and
//! hiding one of them by recency would decide a contested case silently.
//!
//! A term that parses as a UUID names a person id instead: it is the one
//! identifier an operator can copy off a card, and the only way to reach a
//! person the journal holds no values for.
//!
//! Browsing is bounded by the page, not by a refusal: an operator reviewing
//! identities needs to see who exists, and `?cursor=` walks the rest rather
//! than asking them to guess a narrower term.
//!
//! Admin-gated and deliberately NOT visibility-filtered: this is the operator
//! surface, and the seeded operator sits outside the org chart on purpose.

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
use super::error::PersonSearchError;
use super::gate::require_admin;
use super::listing::{self, CursorRejected};
use super::resolution::PersonSummaryResponse;
use crate::domain::person_card;
use crate::domain::resolution::EXCLUDED_PERSON;
use crate::infra::db::person_listing::{self, After, PersonListRow};
use crate::infra::db::persons_repo;

const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 100;
/// A picker query is a human typing, not a batch filter.
const MAX_TERMS: usize = 8;
/// Generous for anything a human pastes into a picker, and a hard ceiling on
/// what each LIKE probe of the journal scan has to compare against.
const MAX_QUERY_CHARS: usize = 200;

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    // Signed so a negative `?limit=` clamps to 1 (parity with the other
    // listings) rather than failing query deserialization.
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// Where the next page resumes. Opaque on the wire; the shape is ours.
#[derive(Debug, Serialize, Deserialize)]
struct PageKey {
    order_key: String,
    person_id: Uuid,
}

impl listing::PagePosition for PageKey {
    const KIND: &'static str = "persons";
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PersonListResponse {
    pub items: Vec<PersonSummaryResponse>,
    /// Pass back as `?cursor=` for the next page; absent on the last one. Only
    /// valid for the query that issued it — narrowing the terms starts over.
    pub next_cursor: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonListResponse {}

/// `GET /v1/persons` — one page of the tenant's persons, narrowed by `?q=`.
pub async fn search_persons(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(params): Query<SearchParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state.db, &ctx).await?;
    let tenant = ctx.subject_tenant_id();

    let terms = search_terms(params.q.as_deref())?;
    let limit = listing::clamp_limit(params.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let (named, values) = partition_terms(&terms);

    let query = terms.join(" ");
    let resume = resume_from(params.cursor.as_deref(), tenant, &query)?;

    // Terms that named nobody must answer nobody. Without this the page would
    // fall through to the unfiltered listing and hand back the whole tenant.
    let rows = if names_nobody(&terms, &named, &values) {
        Vec::new()
    } else {
        page_of_persons(&state, tenant, &values, &named, resume.as_ref(), limit).await?
    };

    let (rows, next_cursor) = cut_to_page(rows, limit, tenant, &query)?;
    let items = hydrate(&state, tenant, &rows).await?;

    Ok(Json(PersonListResponse { items, next_cursor }))
}

/// One page plus the over-fetched truncation probe.
async fn page_of_persons(
    state: &AppState,
    tenant: Uuid,
    values: &[String],
    named: &[Uuid],
    resume: Option<&PageKey>,
    limit: u64,
) -> Result<Vec<PersonListRow>, CanonicalError> {
    let after = resume.map(|key| After {
        order_key: &key.order_key,
        person_id: key.person_id,
    });

    person_listing::list_persons(&state.db, tenant, values, named, after, limit + 1)
        .await
        .map_err(|e| read_err(&e))
}

/// Drop the probe row and, when it was there, mint the cursor that resumes
/// after the last row actually served.
fn cut_to_page(
    mut rows: Vec<PersonListRow>,
    limit: u64,
    tenant: Uuid,
    query: &str,
) -> Result<(Vec<PersonListRow>, Option<String>), CanonicalError> {
    if rows.len() <= usize::try_from(limit).unwrap_or(usize::MAX) {
        return Ok((rows, None));
    }
    rows.pop();

    let next = rows
        .last()
        .map(|last| {
            listing::encode_cursor(
                tenant,
                query,
                &PageKey {
                    order_key: last.order_key.clone(),
                    person_id: last.person_id,
                },
            )
        })
        .transpose()
        .map_err(|e| {
            tracing::error!(error = %e, "failed to issue a person page cursor");
            CanonicalError::internal("failed to list persons").create()
        })?;

    Ok((rows, next))
}

/// Cards for the page, in the order the listing put them.
async fn hydrate(
    state: &AppState,
    tenant: Uuid,
    rows: &[PersonListRow],
) -> Result<Vec<PersonSummaryResponse>, CanonicalError> {
    let ids: Vec<Uuid> = rows.iter().map(|row| row.person_id).collect();
    let cards = persons_repo::person_cards(&state.db, tenant, &ids)
        .await
        .map_err(|e| read_err(&e))?;

    let mut items: Vec<PersonSummaryResponse> = person_card::in_requested_order(&ids, &cards)
        .into_iter()
        .map(PersonSummaryResponse::from)
        .collect();

    // A picker is where the wrong person gets chosen, so a person who exists
    // only because somebody signed in must say so here of all places.
    super::resolution::mark_provisional(state, tenant, &mut items).await?;
    Ok(items)
}

fn resume_from(
    cursor: Option<&str>,
    tenant: Uuid,
    query: &str,
) -> Result<Option<PageKey>, CanonicalError> {
    let Some(cursor) = cursor.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };

    listing::decode_cursor::<PageKey>(cursor, tenant, query)
        .map(Some)
        .map_err(|rejected: CursorRejected| invalid("cursor", rejected.message()))
}

/// The caller typed something, and none of it can name a person.
fn names_nobody(terms: &[String], named: &[Uuid], values: &[String]) -> bool {
    !terms.is_empty() && named.is_empty() && values.is_empty()
}

/// A term that parses as a UUID names a person id; everything else is matched
/// against observed values.
///
/// Without this the one identifier an operator can copy off a card finds
/// nothing, and a person the journal holds no attributes for — minted at first
/// sign-in, before the resolver attaches the roster's name — cannot be found at
/// all, since a value search has no value to match.
fn partition_terms(terms: &[String]) -> (Vec<Uuid>, Vec<String>) {
    let mut named = Vec::new();
    let mut values = Vec::new();
    for term in terms {
        match Uuid::parse_str(term) {
            // The excluded-person sentinel is not a person; naming it finds
            // nobody rather than serving the row every exclusion appends to.
            Ok(id) if id != EXCLUDED_PERSON => named.push(id),
            Ok(_) => {}
            Err(_) => values.push(term.clone()),
        }
    }
    (named, values)
}

/// Split `q` into terms: non-empty, whitespace-separated, capped in count and
/// total length. An absent or blank `q` is the whole roster, not an error.
fn search_terms(q: Option<&str>) -> Result<Vec<String>, CanonicalError> {
    let q = q.unwrap_or_default();
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(invalid(
            "q",
            &format!("at most {MAX_QUERY_CHARS} characters are accepted"),
        ));
    }

    let terms: Vec<String> = q.split_whitespace().map(str::to_owned).collect();

    if terms.len() > MAX_TERMS {
        return Err(invalid(
            "q",
            &format!("at most {MAX_TERMS} search terms are accepted"),
        ));
    }
    Ok(terms)
}

fn invalid(field: &str, message: &str) -> CanonicalError {
    PersonSearchError::invalid_argument()
        .with_field_violation(field, message, "INVALID")
        .create()
}

fn read_err(e: &anyhow::Error) -> CanonicalError {
    tracing::error!(error = %e, "person listing failed");
    CanonicalError::internal("failed to list persons").create()
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    type R = Result<(), Box<dyn Error>>;

    fn row(order_key: &str) -> PersonListRow {
        PersonListRow {
            person_id: Uuid::now_v7(),
            order_key: order_key.to_owned(),
        }
    }

    #[test]
    fn terms_split_on_any_whitespace_and_cap() -> R {
        let terms =
            search_terms(Some("  alice   example.com ")).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert_eq!(terms, vec!["alice".to_owned(), "example.com".to_owned()]);

        assert!(
            search_terms(Some("a b c d e f g h i")).is_err(),
            "over the term cap refused"
        );
        assert!(
            search_terms(Some(&"x".repeat(201))).is_err(),
            "over the length cap refused"
        );
        Ok(())
    }

    #[test]
    fn an_absent_or_blank_query_lists_the_roster_rather_than_failing() -> R {
        // The page bounds the answer, so there is nothing to protect against
        // by refusing: a UI that races its debounce gets a first page, not the
        // tenant.
        for q in [None, Some(""), Some("   ")] {
            let terms = search_terms(q).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            assert!(terms.is_empty(), "should browse: {q:?}");
        }
        Ok(())
    }

    #[test]
    fn a_uuid_term_names_a_person_while_the_rest_match_values() {
        let terms = vec![
            "019e27bc-dec6-7773-b1e7-820ea2624b1b".to_owned(),
            "ann".to_owned(),
        ];

        let (named, values) = partition_terms(&terms);

        assert_eq!(named.len(), 1, "the id is a name, not a value to match");
        assert_eq!(values, vec!["ann".to_owned()]);
    }

    #[test]
    fn the_excluded_sentinel_names_nobody() {
        // It accumulates a journal row per exclusion, so it exists in the
        // table — and it is still not a person the picker may offer.
        let terms = vec![EXCLUDED_PERSON.to_string()];

        let (named, values) = partition_terms(&terms);

        assert!(named.is_empty());
        assert!(values.is_empty(), "not matched as a value either");
        assert!(
            names_nobody(&terms, &named, &values),
            "a query that named nobody must answer nobody, not everybody"
        );
    }

    const TENANT: Uuid = Uuid::from_u128(0x7e_11a7);

    #[test]
    fn browsing_is_not_a_query_that_named_nobody() {
        assert!(
            !names_nobody(&[], &[], &[]),
            "an empty query is the roster, not an empty answer"
        );
    }

    #[test]
    fn a_full_page_offers_the_next_one_and_a_short_page_ends_the_walk() -> R {
        let full = vec![row("0alice"), row("0bob"), row("0carol")];
        let (served, next) = cut_to_page(full, 2, TENANT, "")?;
        assert_eq!(served.len(), 2, "the probe row is never served");
        assert!(next.is_some(), "a probe row means another page exists");

        let short = vec![row("0alice")];
        let (served, next) = cut_to_page(short, 2, TENANT, "")?;
        assert_eq!(served.len(), 1);
        assert!(next.is_none(), "no probe row is the end of the list");
        Ok(())
    }

    #[test]
    fn the_next_cursor_resumes_after_the_last_row_served() -> R {
        let rows = vec![row("0alice"), row("0bob"), row("0carol")];
        let second = rows[1].clone();

        let (_, next) = cut_to_page(rows, 2, TENANT, "")?;
        let cursor = next.ok_or("expected a next page")?;
        let key: PageKey = listing::decode_cursor(&cursor, TENANT, "")
            .map_err(|rejected| rejected.message().to_owned())?;

        assert_eq!(key.order_key, second.order_key);
        assert_eq!(key.person_id, second.person_id);
        Ok(())
    }

    #[test]
    fn a_cursor_is_refused_once_the_query_changes() {
        let rows = vec![row("0alice"), row("0bob")];
        let cursor = cut_to_page(rows, 1, TENANT, "iva")
            .ok()
            .and_then(|(_, next)| next)
            .unwrap_or_default();

        assert!(
            resume_from(Some(&cursor), TENANT, "ivan").is_err(),
            "resuming a narrowed search mid-alphabet would skip people"
        );
        assert!(
            resume_from(Some(&cursor), TENANT, "iva").is_ok(),
            "the query that issued it still walks"
        );
    }

    #[test]
    fn an_absent_or_blank_cursor_starts_at_the_first_page() -> R {
        for cursor in [None, Some(""), Some("  ")] {
            assert!(
                resume_from(cursor, TENANT, "")
                    .map_err(|e| anyhow::anyhow!("{e:?}"))?
                    .is_none(),
                "should start over: {cursor:?}"
            );
        }
        Ok(())
    }
}
