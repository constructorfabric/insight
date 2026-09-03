use std::collections::BTreeMap;
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
use super::gate::{require_admin, require_caller};
use super::listing::{self, CursorRejected};
use crate::infra::db::people_listing::{
    self, After, ListQuery, PeopleListingError, PersonListRow, Restrict, VisibleTo,
};

const DEFAULT_LIMIT: u64 = 50;
const MAX_LIMIT: u64 = 500;

#[derive(Debug, Deserialize)]
pub struct PeopleParams {
    pub visibility: Option<String>,
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeopleVisibility {
    Caller,
    Tenant,
}

impl PeopleVisibility {
    fn parse(value: Option<&str>) -> Result<Self, CanonicalError> {
        match value {
            None | Some("caller") => Ok(Self::Caller),
            Some("tenant") => Ok(Self::Tenant),
            Some(_) => Err(invalid("visibility", "must be `caller` or `tenant`")),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Caller => "caller",
            Self::Tenant => "tenant",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PageKey {
    order_key: String,
    person_id: Uuid,
}

impl listing::PagePosition for PageKey {
    const KIND: &'static str = "people";
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PeopleListItemResponse {
    pub person_id: Uuid,
    /// Source-provided display name, or the available source-provided name
    /// parts joined together.
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub username: Option<String>,
    pub email: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub manager_person_id: Option<Uuid>,
}

impl From<PersonListRow> for PeopleListItemResponse {
    fn from(person: PersonListRow) -> Self {
        Self {
            person_id: person.person_id,
            display_name: person.display_name,
            first_name: person.first_name,
            last_name: person.last_name,
            username: person.username,
            email: person.email,
            attributes: person.attributes,
            manager_person_id: person.manager_person_id,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PeopleListResponse {
    pub items: Vec<PeopleListItemResponse>,
    pub next_cursor: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for PeopleListResponse {}

pub async fn list_people(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(params): Query<PeopleParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    let caller = require_caller(&ctx)?;
    let tenant = ctx.subject_tenant_id();
    let visibility = PeopleVisibility::parse(params.visibility.as_deref())?;
    if visibility == PeopleVisibility::Tenant {
        require_admin(&state.db, &ctx).await?;
    }

    let terms = search_terms(params.q.as_deref())?;
    let (named, values) = listing::partition_person_terms(&terms);
    let limit = listing::clamp_limit(params.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let query = cursor_query(visibility, caller, &terms.join(" "));
    let resume = resume_from(params.cursor.as_deref(), tenant, &query)?;
    let restrict = Restrict {
        visible_to: (visibility == PeopleVisibility::Caller).then_some(VisibleTo {
            viewer_person_id: caller,
            org_source_type: &state.config.org_chart_source_type,
            policy: state.config.visibility_policy,
        }),
    };

    let rows = if listing::person_terms_name_nobody(&terms, &named, &values) {
        Vec::new()
    } else {
        people_listing::list_persons(
            &state.db,
            tenant,
            ListQuery {
                org_chart_source_type: &state.config.org_chart_source_type,
                terms: &values,
                person_ids: &named,
                restrict,
                after: resume.as_ref().map(|key| After {
                    order_key: &key.order_key,
                    person_id: key.person_id,
                }),
                limit: limit + 1,
            },
        )
        .await
        .map_err(|error| read_error(&error))?
    };

    let (rows, next_cursor) =
        listing::cut_to_page(rows, limit, tenant, &query, |row: &PersonListRow| PageKey {
            order_key: row.order_key.clone(),
            person_id: row.person_id,
        })
        .map_err(|error| {
            tracing::error!(%error, "failed to issue a people page cursor");
            CanonicalError::internal("failed to list people").create()
        })?;

    let items = rows.into_iter().map(PeopleListItemResponse::from).collect();
    Ok(Json(PeopleListResponse { items, next_cursor }))
}

fn search_terms(q: Option<&str>) -> Result<Vec<String>, CanonicalError> {
    listing::search_terms(q.unwrap_or_default()).map_err(|message| invalid("q", &message))
}

fn cursor_query(visibility: PeopleVisibility, caller: Uuid, query: &str) -> String {
    format!("{}|{caller}|{query}", visibility.as_str())
}

fn resume_from(
    cursor: Option<&str>,
    tenant: Uuid,
    query: &str,
) -> Result<Option<PageKey>, CanonicalError> {
    listing::resume_from::<PageKey>(cursor, tenant, query)
        .map_err(|rejected: CursorRejected| invalid("cursor", rejected.message()))
}

fn invalid(field: &str, message: &str) -> CanonicalError {
    PersonSearchError::invalid_argument()
        .with_field_violation(field, message, "INVALID")
        .create()
}

fn read_error(error: &PeopleListingError) -> CanonicalError {
    tracing::error!(%error, "people listing failed");
    CanonicalError::internal("failed to list people").create()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_defaults_to_caller_and_rejects_unknown_values() {
        assert!(matches!(
            PeopleVisibility::parse(None),
            Ok(PeopleVisibility::Caller)
        ));
        assert!(matches!(
            PeopleVisibility::parse(Some("tenant")),
            Ok(PeopleVisibility::Tenant)
        ));
        assert!(PeopleVisibility::parse(Some("all")).is_err());
    }

    #[test]
    fn cursor_context_is_bound_to_caller_visibility_and_query() {
        let caller = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);

        assert_ne!(
            cursor_query(PeopleVisibility::Caller, caller, "alex"),
            cursor_query(PeopleVisibility::Tenant, caller, "alex")
        );
        assert_ne!(
            cursor_query(PeopleVisibility::Caller, caller, "alex"),
            cursor_query(PeopleVisibility::Caller, other, "alex")
        );
        assert_ne!(
            cursor_query(PeopleVisibility::Caller, caller, "alex"),
            cursor_query(PeopleVisibility::Caller, caller, "sam")
        );
    }
}
