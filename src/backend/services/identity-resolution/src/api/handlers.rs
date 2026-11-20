use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query};
use axum::response::IntoResponse;
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::AppState;
use super::canonical_json::CanonicalJson;
use super::error::ProfileError;
use super::gate::{require_caller, require_service};
use crate::domain::login_bootstrap;
use crate::domain::profile::{
    BatchProfilesRequest, ParentProjection, PersonResponse, ResolveProfileRequest, assemble_person,
    assemble_profile, latest_values,
};
use crate::domain::profile_batch::{self, BatchProfilesError};
use crate::infra::db::{persons_repo, resolution_repo, roles_repo, subchart_repo};

pub async fn resolve_profile(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<ResolveProfileRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let caller = require_caller(&ctx)?;
    let candidate_ids = resolve_person_ids(&state, tenant, &req).await?;
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

pub async fn batch_profiles(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<BatchProfilesRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let caller = require_caller(&ctx)?;

    let response = profile_batch::resolve_batch_profiles(
        &state.db,
        tenant,
        caller,
        req,
        &state.config.org_chart_source_type,
        state.config.visibility_policy,
    )
    .await
    .map_err(batch_profile_read_error)?;

    Ok(Json(response))
}

#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn batch_profile_read_error(error: BatchProfilesError) -> CanonicalError {
    tracing::error!(%error, "batch profile read failed");
    CanonicalError::internal("profile assembly failed").create()
}

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
            state.config.visibility_policy,
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

#[derive(Debug, Serialize)]
struct InternalPersonResponse {
    value_type: String,
    value: String,
    insight_source_type: &'static str,
    insight_source_id: Uuid,
}

#[derive(Debug, serde::Deserialize)]
pub struct InternalByExternalIdQuery {
    source_type: String,
    external_id: String,
}

pub async fn internal_person_by_external_id(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(query): Query<InternalByExternalIdQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_service(&ctx)?;

    let source_type = query.source_type.trim();
    let external_id = query.external_id.trim();
    if source_type.is_empty() {
        return Err(ProfileError::invalid_argument()
            .with_field_violation("source_type", "source_type must not be empty", "REQUIRED")
            .create());
    }
    if external_id.is_empty() {
        return Err(ProfileError::invalid_argument()
            .with_field_violation("external_id", "external_id must not be empty", "REQUIRED")
            .create());
    }

    let person_id = lookup_by_external_id(&state, source_type, external_id)
        .await?
        .ok_or_else(|| {
            ProfileError::not_found(format!(
                "person with source_type '{source_type}' external_id '{external_id}' not found"
            ))
            .with_resource(external_id.to_owned())
            .create()
        })?;

    Ok(Json(person_response(external_id, person_id)))
}

#[derive(Debug, serde::Deserialize)]
pub struct InternalProvisionRequest {
    source_type: String,
    external_id: String,
    tenant_id: Uuid,
}

pub async fn internal_provision_person(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<InternalProvisionRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_service(&ctx)?;

    let principal =
        login_bootstrap::parse_principal(&req.source_type, &req.external_id, req.tenant_id)
            .map_err(|refusal| refused(refusal, &req))?;
    let (source_type, external_id) = (principal.source_type, principal.external_id);
    let tenant = login_bootstrap::provisioning_tenant(
        &state.config.tenant_default_id,
        principal.asserted_tenant,
    )
    .map_err(|refusal| refused(login_bootstrap::Refusal::Tenant(refusal), &req))?;

    if let Some(person_id) = lookup_by_external_id(&state, source_type, external_id).await? {
        return Ok(Json(person_response(external_id, person_id)));
    }

    let observed = super::resolution::evidence_reader(&state)
        .observed_account(source_type, external_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "login bootstrap: connector evidence lookup failed");
            CanonicalError::internal("lookup failed").create()
        })?
        .ok_or_else(|| {
            ProfileError::not_found(format!(
                "no connector has observed source_type '{source_type}' external_id '{external_id}'"
            ))
            .with_resource(external_id.to_owned())
            .create()
        })?;

    let row = login_bootstrap::decide(principal, &observed, tenant, chrono::Utc::now().naive_utc())
        .map_err(|refusal| refused(refusal, &req))?;

    let minted = resolution_repo::append_binding_if_unbound(&state.db, tenant, &row)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "login bootstrap: binding write failed");
            CanonicalError::internal("provisioning failed").create()
        })?;

    let person_id = lookup_by_external_id(&state, source_type, external_id)
        .await?
        .ok_or_else(|| {
            tracing::warn!(
                target: "audit",
                event = "login_bootstrap_refused_decided_account",
                source_type,
                external_id,
                "the account is already decided as not-a-person; no login identity for it"
            );
            ProfileError::not_found(format!(
                "source_type '{source_type}' external_id '{external_id}' resolves to no person"
            ))
            .with_resource(external_id.to_owned())
            .create()
        })?;

    if minted {
        tracing::info!(
            target: "audit",
            event = "login_bootstrap_person_provisioned",
            source_type,
            external_id,
            person_id = %person_id,
            "minted a person for an authenticated principal the roster already lists"
        );
    }

    Ok(Json(person_response(external_id, person_id)))
}

fn refused(refusal: login_bootstrap::Refusal, req: &InternalProvisionRequest) -> CanonicalError {
    use login_bootstrap::{Refusal, TenantRefusal};

    let resource = req.external_id.trim().to_owned();
    match refusal {
        Refusal::Invalid { field, message } => ProfileError::invalid_argument()
            .with_field_violation(field, message, "INVALID")
            .create(),
        Refusal::Tenant(TenantRefusal::Unconfigured) => ProfileError::failed_precondition()
            .with_precondition_violation(
                "tenant",
                "provisioning needs the service's default tenant to be configured",
                "tenant_unconfigured",
            )
            .create(),
        Refusal::Tenant(TenantRefusal::Mismatch) => ProfileError::invalid_argument()
            .with_field_violation(
                "tenant_id",
                "tenant_id is not the tenant this journal is keyed by",
                "TENANT_MISMATCH",
            )
            .create(),
        Refusal::Closed => ProfileError::not_found(format!("'{resource}' is closed at its source"))
            .with_resource(resource)
            .create(),
        Refusal::Addressed => ProfileError::not_found(format!(
            "'{resource}' carries an address; identity resolution links it, \
             so there is nothing to bootstrap"
        ))
        .with_resource(resource)
        .create(),
    }
}

async fn lookup_by_external_id(
    state: &AppState,
    source_type: &str,
    external_id: &str,
) -> Result<Option<Uuid>, CanonicalError> {
    persons_repo::resolve_person_id_by_source_any_tenant(&state.db, source_type, external_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "internal by-external-id lookup failed");
            CanonicalError::internal("lookup failed").create()
        })
}

fn person_response(external_id: &str, person_id: Uuid) -> InternalPersonResponse {
    InternalPersonResponse {
        value_type: "id".to_owned(),
        value: external_id.to_owned(),
        insight_source_type: "person",
        insight_source_id: person_id,
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct InternalActiveRolesQuery {
    person_id: Uuid,
}

#[derive(Debug, Serialize)]
struct InternalActiveRolesResponse {
    person_id: Uuid,
    roles: Vec<String>,
}

pub async fn internal_person_active_roles(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(query): Query<InternalActiveRolesQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_service(&ctx)?;

    if query.person_id.is_nil() {
        return Err(ProfileError::invalid_argument()
            .with_field_violation("person_id", "person_id must not be nil", "REQUIRED")
            .create());
    }
    let tenant = ctx.subject_tenant_id();
    if tenant.is_nil() {
        return Err(ProfileError::failed_precondition()
            .with_precondition_violation(
                "tenant_id",
                "reading role grants needs the caller's tenant",
                "tenant_unresolved",
            )
            .create());
    }

    let roles = roles_repo::active_roles_of_person(&state.db, tenant, query.person_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "internal active-roles lookup failed");
            CanonicalError::internal("lookup failed").create()
        })?;

    Ok(Json(InternalActiveRolesResponse {
        person_id: query.person_id,
        roles: roles.into_iter().map(|role| role.name).collect(),
    }))
}

#[derive(Debug, serde::Deserialize)]
pub struct InternalByEmailOverrideQuery {
    email: String,
}

pub async fn internal_person_by_email_override(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(query): Query<InternalByEmailOverrideQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_service(&ctx)?;

    let email = query.email.trim();
    if email.is_empty() {
        return Err(ProfileError::invalid_argument()
            .with_field_violation("email", "email must not be empty", "REQUIRED")
            .create());
    }

    let person_id = persons_repo::resolve_person_id_by_email_any_tenant(&state.db, email)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "internal by-email-override lookup failed");
            CanonicalError::internal("lookup failed").create()
        })?
        .ok_or_else(|| {
            ProfileError::not_found(format!("person with email '{email}' not found"))
                .with_resource(email.to_owned())
                .create()
        })?;

    Ok(Json(InternalPersonResponse {
        value_type: "email".to_owned(),
        value: email.to_owned(),
        insight_source_type: "person",
        insight_source_id: person_id,
    }))
}

#[derive(Debug, serde::Deserialize)]
pub struct InternalByRosterEmailQuery {
    email: String,
}

pub async fn internal_person_by_roster_email(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(query): Query<InternalByRosterEmailQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_service(&ctx)?;

    let asked = login_bootstrap::parse_roster_email(
        &query.email,
        &state.config.roster_source_type,
        ctx.subject_tenant_id(),
    )
    .map_err(refused_roster_email)?;

    let candidates = persons_repo::resolve_person_ids_by_roster_email(
        &state.db,
        asked.tenant_id,
        asked.source_type,
        asked.address,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "internal by-roster-email lookup failed");
        CanonicalError::internal("lookup failed").create()
    })?;

    let resolved = login_bootstrap::choose_roster_email_match(&candidates)
        .ok_or_else(|| no_person_states(&asked))?;
    audit_contested_roster_email(&asked, &resolved);

    Ok(Json(InternalPersonResponse {
        value_type: "email".to_owned(),
        value: asked.address.to_owned(),
        insight_source_type: "person",
        insight_source_id: resolved.person_id,
    }))
}

fn no_person_states(asked: &login_bootstrap::RosterEmail<'_>) -> CanonicalError {
    ProfileError::not_found(format!(
        "no person holding a live {} account states email '{}'",
        asked.source_type, asked.address
    ))
    .with_resource(asked.address.to_owned())
    .create()
}

pub(super) fn audit_contested_roster_email(
    asked: &login_bootstrap::RosterEmail<'_>,
    resolved: &login_bootstrap::RosterEmailMatch,
) {
    if resolved.candidates <= 1 {
        return;
    }
    tracing::warn!(
        target: "audit",
        event = "login_roster_email_ambiguous",
        tenant_id = %asked.tenant_id,
        source_type = %asked.source_type,
        candidates = resolved.candidates,
        resolved_person_id = %resolved.person_id,
        "several persons state this roster address; resolving to the newest observation"
    );
}

pub(super) fn refused_roster_email(refusal: login_bootstrap::RosterEmailRefusal) -> CanonicalError {
    use login_bootstrap::RosterEmailRefusal as R;
    match refusal {
        R::AddressMissing => {
            tracing::warn!(
                "by-roster-email called with an empty address — the caller resolved no email claim"
            );
            ProfileError::invalid_argument()
                .with_field_violation("email", "email must not be empty", "REQUIRED")
                .create()
        }
        R::RosterUnconfigured => {
            tracing::warn!(
                "by-roster-email called with no roster_source_type configured — refusing to \
                 match a login address against every source"
            );
            ProfileError::failed_precondition()
                .with_precondition_violation(
                    "roster_source_type",
                    "resolving a login by address needs the roster source to be configured",
                    "roster_source_type_unconfigured",
                )
                .create()
        }
        R::TenantUnresolved => {
            tracing::warn!(
                "by-roster-email called with no tenant on the caller's token — refusing to \
                 match a login address across tenants"
            );
            ProfileError::failed_precondition()
                .with_precondition_violation(
                    "tenant_id",
                    "resolving a login by address needs the caller's tenant",
                    "tenant_unresolved",
                )
                .create()
        }
    }
}

async fn resolve_person_ids(
    state: &AppState,
    tenant: Uuid,
    req: &ResolveProfileRequest,
) -> Result<Vec<Uuid>, CanonicalError> {
    let value_type = req.value_type.trim();

    if value_type.is_empty() {
        return Err(ProfileError::invalid_argument()
            .with_field_violation("value_type", "value_type is required", "REQUIRED")
            .create());
    }
    if value_type != "email" && value_type != "id" && value_type != "person_id" {
        return Err(ProfileError::invalid_argument()
            .with_field_violation(
                "value_type",
                "value_type must be 'email', 'id' or 'person_id'",
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

    if value_type == "person_id" {
        return resolve_person_id_mode(state, tenant, req).await;
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

async fn resolve_person_id_mode(
    state: &AppState,
    tenant: Uuid,
    req: &ResolveProfileRequest,
) -> Result<Vec<Uuid>, CanonicalError> {
    if req.insight_source_type.is_some() || req.insight_source_id.is_some() {
        return Err(ProfileError::invalid_argument()
            .with_field_violation(
                "insight_source_type",
                "insight_source_type / insight_source_id must be null for value_type='person_id'",
                "INVALID",
            )
            .create());
    }

    let person_id = Uuid::parse_str(req.value.trim())
        .ok()
        .filter(|person_id| !person_id.is_nil())
        .ok_or_else(|| {
            ProfileError::invalid_argument()
                .with_field_violation(
                    "value",
                    "value must be a person UUID for value_type='person_id'",
                    "INVALID",
                )
                .create()
        })?;

    let exists = persons_repo::person_exists(&state.db, tenant, person_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "resolve by person id failed");
            CanonicalError::internal("profile resolution failed").create()
        })?;
    Ok(if exists { vec![person_id] } else { Vec::new() })
}

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

    #[test]
    fn internal_person_response_wire_shape() -> anyhow::Result<()> {
        let body = InternalPersonResponse {
            value_type: "email".to_owned(),
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

    #[test]
    fn internal_active_roles_wire_shape() -> anyhow::Result<()> {
        let body = InternalActiveRolesResponse {
            person_id: Uuid::from_u128(1),
            roles: vec!["admin".to_owned(), "previews-admin".to_owned()],
        };
        let json = serde_json::to_value(&body)?;
        assert_eq!(json["person_id"], "00000000-0000-0000-0000-000000000001");
        assert_eq!(
            json["roles"],
            serde_json::json!(["admin", "previews-admin"])
        );
        Ok(())
    }
}
