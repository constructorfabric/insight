//! Route handlers (thin controllers — DTOs + assembly live in `crate::domain`).
//!
//! `/health` + `/healthz` + `/docs` are provided by the api-gateway host gear,
//! so this service defines no health handler.

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
use crate::infra::db::{persons_repo, resolution_repo, subchart_repo};

/// `POST /v1/profiles` — resolve one identity (email or source-native id) to a
/// person, then assemble the profile.
///
/// 0 matches → 404; >1 → 409. (The gears canonical model has no 422, so an
/// ambiguous profile maps to `aborted`/409, the same as the roles /
/// person-roles guards.)
pub async fn resolve_profile(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<ResolveProfileRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let caller = require_caller(&ctx)?;
    let candidate_ids = resolve_person_ids(&state, tenant, &req).await?;
    // Visibility gate: a caller may only resolve profiles they can see. Filter BEFORE deciding
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
            // Resolver returned an id but hydration found no rows →
            // not-found. Practically unreachable.
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
            // fix the data: the gears canonical model has no structured
            // payload, so they go in the text.
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

/// Wire shape of the internal S2S lookup response:
/// `{ value_type, value, insight_source_type, insight_source_id }`.
#[derive(Debug, Serialize)]
struct InternalPersonResponse {
    value_type: String,
    value: String,
    insight_source_type: &'static str,
    insight_source_id: Uuid,
}

/// Query params for `GET /internal/persons/by-external-id`.
#[derive(Debug, serde::Deserialize)]
pub struct InternalByExternalIdQuery {
    source_type: String,
    external_id: String,
}

/// `GET /internal/persons/by-external-id?source_type=...&external_id=...` —
/// SERVICE-ONLY any-tenant `person_id` resolution for the LOGIN BOOTSTRAP
/// ONLY: scoped to the configured `IdP`'s `source_type` (e.g. `ms-entra`) +
/// its source-native external user id (e.g. the Entra `oid` claim). NEVER
/// resolves by email — the two address-matching routes are SEPARATE
/// ([`internal_person_by_roster_email`] for an install configured to resolve
/// logins by address, [`internal_person_by_email_override`] for view-as), so a
/// login that somehow carries no external id has no path that silently falls
/// through to either. Which route the authenticator calls is decided by its
/// `idp.resolve_by` config, once, at the top of the login — never by what a
/// given token happens to carry.
///
/// Deliberately bypasses the tenant + visibility gates the public
/// `/v1/profiles` enforces: at login neither a tenant nor a caller identity
/// exists yet. Still fail-closed — a valid gateway JWT is required (host
/// authn), and a non-service principal (`subject_type != "service"`, the
/// gears mapping of the `sub_type` claim) gets 403. Registered as a raw
/// route so it stays out of the public OpenAPI. This is the login-bootstrap
/// lookup: it resolves by external id, never by email.
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

/// Body for `POST /internal/persons/provision`.
#[derive(Debug, serde::Deserialize)]
pub struct InternalProvisionRequest {
    source_type: String,
    external_id: String,
    /// The tenant the `id_token` asserted. A read can stay tenant-agnostic; a
    /// write cannot, and at login there is no caller context to infer it from.
    tenant_id: Uuid,
}

/// `POST /internal/persons/provision` — SERVICE-ONLY login bootstrap that
/// MINTS a person when the journal has no binding for this IdP principal yet.
/// Same contract and gate as [`internal_person_by_external_id`], and the same
/// response shape, so the caller can treat the two identically.
///
/// Why this exists: the login-bootstrap row is otherwise written only by the
/// nightly persons-seed, which links a person by e-mail and skips an account
/// that carries none. A member of the IdP's roster with no published address
/// is therefore refused at login until an operator binds them by hand.
///
/// It mints only for an account a connector has ALREADY OBSERVED, and reuses
/// that observation's `insight_source_id`. Both halves matter:
///
/// - the roster stays the authority on who exists, so this is "the IdP
///   authenticated someone the org already lists", never "anyone who reaches
///   the IdP becomes a person";
/// - the persons-seed recognises an account by the whole triple, so a binding
///   written under any other instance id would be invisible to it and the
///   account would stay unbound forever. Matching the observed id is what
///   makes the next batch run ADOPT this person rather than mint a second.
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
    // Validated before the lookup, not just before the write: a route that
    // answered an existing person for any asserted tenant and refused only a
    // new one would fail intermittently under a misconfigured tenant claim,
    // which is the shape nobody diagnoses.
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

    // Read what is in force, never what was intended. Two interleavings end up
    // here: a racing login wrote first, or an operator decided first —
    // including an exclusion, which the lookup hides and which must read as
    // "no person to enter as" rather than as a fresh mint.
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

/// Map a domain refusal onto the wire. Each is an answer about the principal,
/// so none of them is a 500.
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

/// Query params for `GET /internal/persons/by-email-override`.
#[derive(Debug, serde::Deserialize)]
pub struct InternalByEmailOverrideQuery {
    email: String,
}

/// `GET /internal/persons/by-email-override?email=...` — SERVICE-ONLY
/// any-tenant `person_id` resolution for the authenticator's admin
/// `__override` (view-as, #1941) feature ONLY: an operator types an email to
/// become that person. NEVER used by the login bootstrap (see
/// [`internal_person_by_external_id`], a separate route) — the two are
/// distinct contracts so an empty/absent external id at login can never fall
/// through to this one.
///
/// Same bypass-tenant-gates rationale and fail-closed service-only gate as
/// `by-external-id`. INVARIANT: override-only by contract — the login path
/// never resolves by email.
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

/// Query params for `GET /internal/persons/by-roster-email`.
#[derive(Debug, serde::Deserialize)]
pub struct InternalByRosterEmailQuery {
    email: String,
}

/// `GET /internal/persons/by-roster-email?email=...` — SERVICE-ONLY
/// `person_id` resolution for the login bootstrap of an install that resolves
/// logins by address (`idp.resolve_by = email` on the authenticator). For
/// installs whose IdP has no directory connector of its own: nothing ever seeds
/// a `value_type='id'` row for the provider, so
/// [`internal_person_by_external_id`] can match nobody and every sign-in is
/// refused.
///
/// A THIRD route rather than a parameter on one of the other two, because the
/// separation between them is a security boundary and not a naming choice: each
/// route answers exactly one question, so no absent or empty field can make a
/// login take a resolution path other than the one the install configured.
/// `by-email-override` in particular stays override-only — it matches an address
/// stated by ANY source in ANY tenant, which is the right latitude for an
/// operator typing a name into view-as and far too much for a sign-in.
///
/// Unlike the two any-tenant lookups this one is tenant-SCOPED. The tenant is
/// known by the time a login reaches here: the authenticator refuses a login
/// whose `id_token` named no tenant, and mints the service JWT with that tenant,
/// so it arrives in the `SecurityContext` like any other caller's. An address
/// does not carry the cross-tenant uniqueness a directory id does, so spending
/// the tenant we already have is what keeps one customer's roster from
/// resolving another customer's login.
///
/// Fails closed on every shape it cannot answer: no roster declared, no tenant
/// on the JWT, an empty address, or an address the roster does not state for
/// anyone who still holds a live account under it.
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

/// The roster states this address for nobody who still holds a live account
/// under it — which is also how an excluded account reads from here.
fn no_person_states(asked: &login_bootstrap::RosterEmail<'_>) -> CanonicalError {
    ProfileError::not_found(format!(
        "no person holding a live {} account states email '{}'",
        asked.source_type, asked.address
    ))
    .with_resource(asked.address.to_owned())
    .create()
}

/// Answering a contested address is the install's chosen behaviour; this is the
/// line that makes it auditable rather than silent. The seed refuses to
/// auto-link the same shape, and an operator may have split the two people
/// deliberately.
fn audit_contested_roster_email(
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

/// Map a roster-email refusal to its wire shape, and log the two that mean an
/// install is misconfigured rather than a caller mistaken — each would
/// otherwise surface only as an unexplained refusal for every person.
fn refused_roster_email(refusal: login_bootstrap::RosterEmailRefusal) -> CanonicalError {
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

/// Validate the request and resolve it to candidate `person_id`s.
///
/// Resolution dispatches on `value_type` ("email" across all sources, "id" scoped to one
/// source instance, `person_id` the canonical person itself). Returns the
/// (possibly empty or multi-element) match set — the caller maps 0 → 404,
/// 1 → profile, >1 → 409.
async fn resolve_person_ids(
    state: &AppState,
    tenant: Uuid,
    req: &ResolveProfileRequest,
) -> Result<Vec<Uuid>, CanonicalError> {
    let value_type = req.value_type.trim();

    // Validation order: value_type first, then value, then the source
    // cross-field rules.
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

/// The `value_type='person_id'` mode: the canonical person needs no resolution
/// step, so this only validates the key and confirms the person exists in the
/// tenant. Visibility still applies downstream, so name resolution and metric
/// access answer to one rule.
async fn resolve_person_id_mode(
    state: &AppState,
    tenant: Uuid,
    req: &ResolveProfileRequest,
) -> Result<Vec<Uuid>, CanonicalError> {
    // Cross-field shape matches email's: a person id is tenant-wide, and
    // source scoping is what selects the 'id' mode instead.
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

    // A person exists iff the append-only log holds an observation for it; an
    // unknown id yields no candidate, so the caller answers 404 — the same
    // shape an unknown email takes, and no probe for which ids exist.
    let exists = persons_repo::person_exists(&state.db, tenant, person_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "resolve by person id failed");
            CanonicalError::internal("profile resolution failed").create()
        })?;
    Ok(if exists { vec![person_id] } else { Vec::new() })
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
    /// `insight_source_type`) — the authenticator depends on it verbatim.
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
}
