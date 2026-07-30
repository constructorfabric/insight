//! Route handlers (thin controllers — DTOs + assembly live in `crate::domain`).
//!
//! `/health` + `/healthz` + `/docs` are provided by the api-gateway host gear,
//! so this service defines no health handler.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::response::IntoResponse;
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::AppState;
use super::canonical_json::CanonicalJson;
use super::error::ProfileError;
use super::gate::{require_caller, require_service};
use crate::domain::profile::{
    ParentProjection, PersonResponse, ResolveProfileRequest, assemble_person, assemble_profile,
    latest_values,
};
use crate::infra::db::{persons_repo, subchart_repo};

/// `POST /v1/profiles` — resolve one identity (email or source-native id) to a
/// person, then assemble the profile.
///
/// 0 matches → 404; >1 → 409. (The .NET service returned 422 `ambiguous_profile`;
/// the gears canonical model has no 422, so this maps to `aborted`/409 — an
/// accepted status divergence, same as the roles / person-roles guards.)
pub async fn resolve_profile(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<ResolveProfileRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let caller = require_caller(&ctx)?;
    let candidate_ids = resolve_person_ids(&state, tenant, &req).await?;
    // Visibility gate (parity with .NET `VisibilityService.CanSeeAsync`): a
    // caller may only resolve profiles they can see. Filter BEFORE deciding
    // between not-found / resolved / ambiguous, so a hidden candidate neither
    // leaks its existence through an `AMBIGUOUS_PROFILE` id list nor causes a
    // uniquely-visible candidate to be misreported as ambiguous.
    let person_ids = visible_person_ids(&state, tenant, caller, candidate_ids).await?;

    match person_ids.as_slice() {
        [] => Err(ProfileError::not_found("person not found")
            .with_resource(req.value)
            .create()),
        [person_id] => {
            let observations =
                persons_repo::fetch_person_observations(&state.db, tenant, *person_id)
                    .await
                    .map_err(|e| {
                        tracing::error!(error = %e, "fetch person observations failed");
                        CanonicalError::internal("profile assembly failed").create()
                    })?;
            // Resolver returned an id but hydration found no rows → not-found
            // (matches .NET ProfileLookupService). Practically unreachable.
            if observations.is_empty() {
                return Err(ProfileError::not_found("person not found")
                    .with_resource(req.value.clone())
                    .create());
            }
            let source_ids =
                persons_repo::current_source_ids_for_person(&state.db, tenant, *person_id)
                    .await
                    .map_err(|e| {
                        tracing::error!(error = %e, "fetch source ids failed");
                        CanonicalError::internal("profile assembly failed").create()
                    })?;
            let parent = resolve_parent(&state, tenant, *person_id).await?;
            let subordinates = resolve_subordinates(&state, tenant, *person_id).await?;
            Ok(Json(assemble_profile(
                *person_id,
                tenant,
                observations,
                source_ids,
                parent,
                subordinates,
            )))
        }
        ids => {
            // >1 match: include the resolved ids in the detail so operators can
            // fix the data (the .NET 422 carried a `person_ids` array; the gears
            // canonical model has no structured payload, so they go in the text).
            let list = ids
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            Err(ProfileError::aborted(format!(
                "identity resolves to {} persons: {list}",
                ids.len()
            ))
            .with_reason("AMBIGUOUS_PROFILE")
            .create())
        }
    }
}

/// Narrow `candidate_ids` down to the ones `caller` can see (current state —
/// `valid_at = None`), preserving order. Run before the not-found / resolved /
/// ambiguous decision so a candidate the caller cannot see never surfaces —
/// neither as a false single match nor as an id in the ambiguous-profile list.
async fn visible_person_ids(
    state: &AppState,
    tenant: Uuid,
    caller: Uuid,
    candidate_ids: Vec<Uuid>,
) -> Result<Vec<Uuid>, CanonicalError> {
    let mut visible = Vec::with_capacity(candidate_ids.len());
    for person_id in candidate_ids {
        let can_see = subchart_repo::is_target_in_visible_set(
            &state.db,
            tenant,
            caller,
            person_id,
            &state.config.org_chart_source_type,
            None,
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "profile visibility check failed");
            CanonicalError::internal("profile assembly failed").create()
        })?;
        if can_see {
            visible.push(person_id);
        }
    }
    Ok(visible)
}

/// Wire shape of the internal S2S lookup response. Mirrors the .NET anonymous
/// object `{ value_type, value, insight_source_type, insight_source_id }`.
#[derive(Debug, Serialize)]
struct InternalPersonResponse {
    value_type: &'static str,
    value: String,
    insight_source_type: &'static str,
    insight_source_id: Uuid,
}

/// `GET /internal/persons/by-email/{email}` — SERVICE-ONLY email → `person_id`
/// resolution for the login bootstrap. Deliberately bypasses the tenant +
/// visibility gates the public `/v1/profiles` enforces: at login neither a
/// tenant nor a caller identity exists yet. Still fail-closed — a valid gateway
/// JWT is required (host authn), and a non-service principal
/// (`subject_type != "service"`, the gears mapping of the .NET `sub_type` claim)
/// gets 403. Registered as a raw route so it stays out of the public OpenAPI,
/// matching the .NET `.ExcludeFromDescription()`. Ported from `PersonsEndpoints`.
pub async fn internal_person_by_email(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(email): Path<String>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_service(&ctx)?;

    let person_id = persons_repo::resolve_person_id_by_email_any_tenant(&state.db, &email)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "internal by-email lookup failed");
            CanonicalError::internal("lookup failed").create()
        })?
        .ok_or_else(|| {
            ProfileError::not_found(format!("person with email '{email}' not found"))
                .with_resource(email.clone())
                .create()
        })?;

    Ok(Json(InternalPersonResponse {
        value_type: "email",
        value: email,
        insight_source_type: "person",
        insight_source_id: person_id,
    }))
}

/// Validate the request and resolve it to candidate `person_id`s.
///
/// Validation mirrors the .NET `ResolveProfileRequestValidator`; resolution
/// dispatches on `value_type` ("email" across all sources, "id" scoped to one
/// source instance). Returns the (possibly empty or multi-element) match set —
/// the caller maps 0 → 404, 1 → profile, >1 → 409.
async fn resolve_person_ids(
    state: &AppState,
    tenant: Uuid,
    req: &ResolveProfileRequest,
) -> Result<Vec<Uuid>, CanonicalError> {
    let value_type = req.value_type.trim();

    // Validation order mirrors the .NET FluentValidation declaration order:
    // value_type first, then value, then the source cross-field rules.
    if value_type.is_empty() {
        return Err(ProfileError::invalid_argument()
            .with_field_violation("value_type", "value_type is required", "REQUIRED")
            .create());
    }
    if value_type != "email" && value_type != "id" {
        return Err(ProfileError::invalid_argument()
            .with_field_violation(
                "value_type",
                "value_type must be 'email' or 'id'",
                "INVALID",
            )
            .create());
    }
    if req.value.trim().is_empty() {
        return Err(ProfileError::invalid_argument()
            .with_field_violation("value", "value must not be empty", "INVALID")
            .create());
    }
    if req.value.chars().count() > 320 {
        return Err(ProfileError::invalid_argument()
            .with_field_violation("value", "value must be at most 320 characters", "INVALID")
            .create());
    }

    if value_type == "id" {
        let source_type = req.insight_source_type.as_deref().ok_or_else(|| {
            ProfileError::invalid_argument()
                .with_field_violation(
                    "insight_source_type",
                    "insight_source_type is required for value_type='id'",
                    "REQUIRED",
                )
                .create()
        })?;
        let source_id = req.insight_source_id.ok_or_else(|| {
            ProfileError::invalid_argument()
                .with_field_violation(
                    "insight_source_id",
                    "insight_source_id is required for value_type='id'",
                    "REQUIRED",
                )
                .create()
        })?;
        persons_repo::resolve_person_ids_by_source_id(
            &state.db,
            tenant,
            source_type,
            source_id,
            &req.value,
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "resolve by source id failed");
            CanonicalError::internal("profile resolution failed").create()
        })
    } else {
        // value_type == "email"
        if req.insight_source_type.is_some() || req.insight_source_id.is_some() {
            return Err(ProfileError::invalid_argument()
                .with_field_violation(
                    "insight_source_type",
                    "insight_source_type / insight_source_id must be null for value_type='email'",
                    "INVALID",
                )
                .create());
        }
        persons_repo::resolve_person_ids_by_email(&state.db, tenant, &req.value)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "resolve by email failed");
                CanonicalError::internal("profile resolution failed").create()
            })
    }
}

/// Resolve the person's parent (supervisor) edge from `org_chart`, filtered to
/// the configured source instance, into the projection the assembler writes.
/// Returns `Ok(None)` when the person has no current parent edge on that source.
async fn resolve_parent(
    state: &AppState,
    tenant: Uuid,
    child_person_id: Uuid,
) -> Result<Option<ParentProjection>, CanonicalError> {
    let source_type = &state.config.org_chart_source_type;

    let edges = persons_repo::current_parents_for_child(&state.db, tenant, child_person_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "fetch parent edges failed");
            CanonicalError::internal("profile assembly failed").create()
        })?;
    let Some(edge) = edges.into_iter().find(|e| &e.source_type == source_type) else {
        return Ok(None);
    };

    let parent_obs =
        persons_repo::fetch_person_observations(&state.db, tenant, edge.parent_person_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "fetch parent observations failed");
                CanonicalError::internal("profile assembly failed").create()
            })?;
    let parent_ids =
        persons_repo::current_source_ids_for_person(&state.db, tenant, edge.parent_person_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "fetch parent source ids failed");
                CanonicalError::internal("profile assembly failed").create()
            })?;

    let latest = latest_values(parent_obs);
    // Parent's source-native id on the same source instance as the edge.
    let source_native_id = parent_ids
        .into_iter()
        .find(|s| &s.source_type == source_type && s.source_id == edge.source_id)
        .map(|s| s.value);

    Ok(Some(ParentProjection {
        person_id: edge.parent_person_id,
        email: latest.get("email").cloned(),
        display_name: latest.get("display_name").cloned(),
        source_native_id,
    }))
}

/// Hydrate the recursive subordinates subtree for a resolved profile. The root
/// is pre-seeded into `visited` so a child edge pointing back at it can't loop.
async fn resolve_subordinates(
    state: &AppState,
    tenant: Uuid,
    root_person_id: Uuid,
) -> Result<Vec<PersonResponse>, CanonicalError> {
    if !state.config.expand_subordinates {
        return Ok(Vec::new());
    }
    let mut visited = HashSet::new();
    visited.insert(root_person_id);
    hydrate_children(state, tenant, root_person_id, 0, &mut visited).await
}

/// Expand the direct children of `person_id` (at tree depth `depth`) into person
/// nodes, recursing while below the configured depth cap. Children are the
/// distinct `org_chart` child ids on the configured source, in query order.
///
/// Returns a boxed future: `hydrate_children` and `hydrate_person` are mutually
/// recursive `async fn`s, which Rust cannot size without an explicit `Box::pin`.
fn hydrate_children<'a>(
    state: &'a AppState,
    tenant: Uuid,
    person_id: Uuid,
    depth: usize,
    visited: &'a mut HashSet<Uuid>,
) -> Pin<Box<dyn Future<Output = Result<Vec<PersonResponse>, CanonicalError>> + Send + 'a>> {
    Box::pin(async move {
        if !state.config.expand_subordinates || depth >= state.config.max_depth {
            return Ok(Vec::new());
        }
        let source_type = &state.config.org_chart_source_type;
        let edges = persons_repo::current_children_for_parent(&state.db, tenant, person_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "fetch child edges failed");
                CanonicalError::internal("profile assembly failed").create()
            })?;

        // Distinct child ids on the configured source, preserving query order.
        let mut seen = HashSet::new();
        let child_ids: Vec<Uuid> = edges
            .into_iter()
            .filter(|e| &e.source_type == source_type)
            .map(|e| e.child_person_id)
            .filter(|id| seen.insert(*id))
            .collect();

        let mut subordinates = Vec::new();
        for child_id in child_ids {
            if let Some(node) = hydrate_person(state, tenant, child_id, depth + 1, visited).await? {
                subordinates.push(node);
            }
        }
        Ok(subordinates)
    })
}

/// Build one person node at tree depth `depth`, recursing into its own children.
/// Returns `None` when the person is already on the current path (cycle guard)
/// or has no observations.
fn hydrate_person<'a>(
    state: &'a AppState,
    tenant: Uuid,
    person_id: Uuid,
    depth: usize,
    visited: &'a mut HashSet<Uuid>,
) -> Pin<Box<dyn Future<Output = Result<Option<PersonResponse>, CanonicalError>> + Send + 'a>> {
    Box::pin(async move {
        if !visited.insert(person_id) {
            return Ok(None);
        }
        let observations = persons_repo::fetch_person_observations(&state.db, tenant, person_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "fetch subordinate observations failed");
                CanonicalError::internal("profile assembly failed").create()
            })?;
        if observations.is_empty() {
            return Ok(None);
        }
        let parent = resolve_parent(state, tenant, person_id).await?;
        let subordinates = hydrate_children(state, tenant, person_id, depth, visited).await?;
        Ok(Some(assemble_person(
            person_id,
            observations,
            parent,
            subordinates,
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lock the internal S2S response wire shape (`snake_case` keys + constant
    /// `value_type`/`insight_source_type`) — the login authenticator depends on
    /// it verbatim.
    #[test]
    fn internal_person_response_wire_shape() -> anyhow::Result<()> {
        let body = InternalPersonResponse {
            value_type: "email",
            value: "a@b.com".to_owned(),
            insight_source_type: "person",
            insight_source_id: Uuid::from_u128(1),
        };
        let json = serde_json::to_value(&body)?;
        assert_eq!(json["value_type"], "email");
        assert_eq!(json["value"], "a@b.com");
        assert_eq!(json["insight_source_type"], "person");
        assert_eq!(
            json["insight_source_id"],
            "00000000-0000-0000-0000-000000000001"
        );
        Ok(())
    }
}
