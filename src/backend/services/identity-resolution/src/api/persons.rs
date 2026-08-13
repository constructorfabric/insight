//! Person search — the operator picker over the observation journal.
//!
//! `GET /v1/persons?q=` matches every whitespace-separated term against the
//! caller-tenant persons' CURRENT observed values (the latest observation per
//! person × source × value type — the same window rule `/v1/profiles` resolves
//! by, so search and resolution cannot disagree). Superseded values stop
//! matching their old owner; a value two persons both currently claim returns
//! both — the operator is the disambiguator, and hiding one of them by
//! recency would decide a contested case silently.
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

use super::AppState;
use super::error::PersonSearchError;
use super::gate::require_admin;
use super::resolution::PersonSummaryResponse;
use crate::domain::person_card;
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
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PersonListResponse {
    pub items: Vec<PersonSummaryResponse>,
    /// More persons matched than `limit` allowed — the page is a cut, not the
    /// answer, and the UI should ask for narrower terms. Without this flag a
    /// truncated page reads as "the person does not exist".
    pub truncated: bool,
    /// Wire parity with the other listings: declared, always `null`.
    pub next_cursor: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonListResponse {}

/// `GET /v1/persons` — search persons by their current observed values (admin).
pub async fn search_persons(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(params): Query<SearchParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state.db, &ctx).await?;
    let tenant = ctx.subject_tenant_id();

    let terms = search_terms(params.q.as_deref())?;
    let limit = super::listing::clamp_limit(params.limit, DEFAULT_LIMIT, MAX_LIMIT);

    // Over-fetch by one: the extra row is the truncation probe, never served.
    let mut ids =
        persons_repo::search_persons_by_current_values(&state.db, tenant, &terms, limit + 1)
            .await
            .map_err(|e| read_err(&e))?;
    let truncated = ids.len() > usize::try_from(limit).unwrap_or(usize::MAX);
    if truncated {
        ids.pop();
    }

    let cards = persons_repo::person_cards(&state.db, tenant, &ids)
        .await
        .map_err(|e| read_err(&e))?;
    let mut items: Vec<PersonSummaryResponse> = person_card::in_requested_order(&ids, &cards)
        .into_iter()
        .map(PersonSummaryResponse::from)
        .collect();
    sort_for_display(&mut items);

    Ok(Json(PersonListResponse {
        items,
        truncated,
        next_cursor: None,
    }))
}

/// Split `q` into terms: non-empty, whitespace-separated, capped in count and
/// total length.
fn search_terms(q: Option<&str>) -> Result<Vec<String>, CanonicalError> {
    let q = q.unwrap_or_default();
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(invalid(
            "q",
            &format!("at most {MAX_QUERY_CHARS} characters are accepted"),
        ));
    }

    let terms: Vec<String> = q.split_whitespace().map(str::to_owned).collect();

    if terms.is_empty() {
        return Err(invalid("q", "search terms are required"));
    }
    if terms.len() > MAX_TERMS {
        return Err(invalid(
            "q",
            &format!("at most {MAX_TERMS} search terms are accepted"),
        ));
    }
    Ok(terms)
}

/// Named first, then email-only, then username-only; persons the journal
/// knows by nothing but an id come last. Alphabetical (case-folded) within
/// each band.
fn sort_for_display(items: &mut [PersonSummaryResponse]) {
    items.sort_by_cached_key(|i| {
        (
            i.display_name.is_none(),
            i.display_name.as_deref().map(str::to_lowercase),
            i.email.is_none(),
            i.email.as_deref().map(str::to_lowercase),
            i.username.is_none(),
            i.username.as_deref().map(str::to_lowercase),
            i.person_id,
        )
    });
}

fn invalid(field: &str, message: &str) -> CanonicalError {
    PersonSearchError::invalid_argument()
        .with_field_violation(field, message, "INVALID")
        .create()
}

fn read_err(e: &anyhow::Error) -> CanonicalError {
    tracing::error!(error = %e, "person search failed");
    CanonicalError::internal("failed to search persons").create()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_split_on_any_whitespace_and_cap() -> anyhow::Result<()> {
        let terms =
            search_terms(Some("  alice   example.com ")).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert_eq!(terms, vec!["alice".to_owned(), "example.com".to_owned()]);

        assert!(search_terms(None).is_err(), "absent q refused");
        assert!(search_terms(Some("   ")).is_err(), "blank q refused");
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
}
