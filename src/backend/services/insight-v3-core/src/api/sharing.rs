//! Sharing a dashboard, and handing an orphaned one to a current
//! administrator.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::SecondsFormat;
use serde::Deserialize;
use serde_json::{Value, json};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, Http};
use utoipa::ToSchema;

use super::AppState;
use super::definitions::{DefinitionApiError, OWNER_ONLY};
use super::errors::ApiErrors;
use crate::domain::access::PersonId;
use crate::domain::definition::{DefinitionId, DefinitionKind, MAX_PAGE_LIMIT, Page};
use crate::domain::sharing::{
    Administrators, AdministratorsError, DashboardShares, Orphan, Share, SharingError,
};
use crate::store::identity::{IdentityClient, IdentityError};

#[derive(Debug, Deserialize, ToSchema)]
struct TransferRequest {
    /// The person id of the current administrator to own it from now on.
    to: String,
}

impl toolkit::api::api_dto::RequestApiDto for TransferRequest {}

#[derive(Debug, Default, Deserialize)]
struct Paging {
    limit: Option<u64>,
    offset: Option<u64>,
}

/// Identity, asked with the caller's own authorization.
struct ForwardedAdministrators<'a> {
    identity: &'a IdentityClient,
    authorization: Option<&'a str>,
}

#[async_trait]
impl Administrators for ForwardedAdministrators<'_> {
    async fn holds_admin(&self, person: PersonId) -> Result<bool, AdministratorsError> {
        self.identity
            .holds_admin(self.authorization, person)
            .await
            .map_err(|error| AdministratorsError::Unavailable(error.to_string()))
    }

    async fn current(&self) -> Result<Vec<PersonId>, AdministratorsError> {
        self.identity
            .admins(self.authorization)
            .await
            .map_err(|error| match error {
                IdentityError::Truncated => AdministratorsError::Truncated,
                IdentityError::Transport(_) | IdentityError::Refused(_) => {
                    AdministratorsError::Unavailable(error.to_string())
                }
            })
    }
}

fn denied() -> CanonicalError {
    DefinitionApiError::permission_denied()
        .with_reason(crate::api::ADMIN_ONLY)
        .create()
}

fn conflict(subject: &str, detail: String, kind: &str) -> CanonicalError {
    DefinitionApiError::failed_precondition()
        .with_precondition_violation(subject, detail, kind)
        .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
        .create()
}

fn sharing_error(error: SharingError) -> CanonicalError {
    let detail = error.to_string();

    match error {
        SharingError::NotFound(id) => DefinitionApiError::not_found(detail)
            .with_resource(&id)
            .create(),
        SharingError::NotOwner => DefinitionApiError::permission_denied()
            .with_reason(OWNER_ONLY)
            .create(),
        SharingError::Ownerless => conflict("owner", detail, "ownerless"),
        SharingError::OwnerIsAdministrator => conflict("owner", detail, "owner_is_administrator"),
        SharingError::Moved => conflict("owner", detail, "owner_changed"),
        SharingError::SharedWithOwner | SharingError::TargetNotAdministrator => {
            DefinitionApiError::invalid_field("person_id", detail)
        }
        SharingError::RecipientNotAdministrator => DefinitionApiError::invalid_field("to", detail),
        SharingError::Administrators(source) => {
            tracing::error!(error = %source, "the current administrators could not be read");
            CanonicalError::internal("failed to verify administrators").create()
        }
        SharingError::Store(source) => DefinitionApiError::definition_store_error(source),
    }
}

fn dashboard_id(raw: &str) -> Result<DefinitionId, CanonicalError> {
    DefinitionId::parse(raw)
        .map_err(|_| DefinitionApiError::malformed_id(DefinitionKind::Dashboard, raw))
}

fn person_id(raw: &str, field: &str) -> Result<PersonId, CanonicalError> {
    PersonId::parse(raw)
        .ok_or_else(|| DefinitionApiError::invalid_field(field, "a person id is a UUID".to_owned()))
}

fn share_json(share: &Share) -> Value {
    json!({
        "person_id": share.person.to_string(),
        "granted_by": share.granted_by.to_string(),
        "created_at": share.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
    })
}

fn orphan_json(orphan: &Orphan) -> Value {
    json!({
        "id": orphan.id,
        "name": orphan.name,
        "owner_person_id": orphan.owner.to_string(),
        "updated_at": orphan.updated_at.to_rfc3339_opts(SecondsFormat::Millis, true),
    })
}

fn path_param(name: &str, description: &str) -> ParamSpec {
    ParamSpec::path(name).description(description)
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    router
        .merge(share_routes(openapi, state))
        .merge(transfer_routes(openapi, state))
}

fn dashboard_param() -> ParamSpec {
    path_param("id", "Dashboard id")
}

fn share_routes(openapi: &dyn OpenApiRegistry, state: &Arc<AppState>) -> Router {
    let dashboard = dashboard_param;
    let person = || path_param("person_id", "The person's id");

    let list = OperationBuilder::get("/v1/dashboards/{id}/shares")
        .operation_id("insight_v3_core.dashboards.shares.list")
        .summary("List who the caller's dashboard is shared with")
        .anonymous()
        .exposed()
        .param(dashboard())
        .json_response(
            StatusCode::OK,
            "Each share: the person, who granted it, and when",
        )
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_shares)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let share = OperationBuilder::put("/v1/dashboards/{id}/shares/{person_id}")
        .operation_id("insight_v3_core.dashboards.shares.put")
        .summary("Share the caller's dashboard with a current administrator; sharing again changes nothing")
        .anonymous()
        .exposed()
        .param(dashboard())
        .param(person())
        .no_content_response(StatusCode::NO_CONTENT, "Shared")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(share_dashboard)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let withdraw = OperationBuilder::delete("/v1/dashboards/{id}/shares/{person_id}")
        .operation_id("insight_v3_core.dashboards.shares.delete")
        .summary(
            "Withdraw a share of the caller's dashboard, with that person's pin and folder for it",
        )
        .anonymous()
        .exposed()
        .param(dashboard())
        .param(person())
        .no_content_response(StatusCode::NO_CONTENT, "Not shared")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(withdraw_share)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    Router::new().merge(list).merge(share).merge(withdraw)
}

fn transfer_routes(openapi: &dyn OpenApiRegistry, state: &Arc<AppState>) -> Router {
    let orphaned = OperationBuilder::get("/v1/orphaned-dashboards")
        .operation_id("insight_v3_core.orphaned_dashboards.list")
        .summary("List the dashboards whose owner is no longer an administrator")
        .anonymous()
        .exposed()
        .param(
            ParamSpec::query("limit")
                .description(format!("Page size, 1 to {MAX_PAGE_LIMIT}"))
                .param_type("integer"),
        )
        .param(
            ParamSpec::query("offset")
                .description("Dashboards to skip")
                .param_type("integer"),
        )
        .json_response(
            StatusCode::OK,
            "One page of orphaned dashboards, and how many there are",
        )
        .error_400(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_orphaned)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let transfer = OperationBuilder::post("/v1/dashboards/{id}/transfer")
        .operation_id("insight_v3_core.dashboards.transfer")
        .summary("Hand an orphaned dashboard to a current administrator")
        .anonymous()
        .exposed()
        .param(dashboard_param())
        .json_request::<TransferRequest>(openapi, "Who owns it from now on")
        .no_content_response(StatusCode::NO_CONTENT, "Transferred")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(transfer_dashboard)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    Router::new().merge(orphaned).merge(transfer)
}

async fn list_shares(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let administrators = forwarded(&state, &headers);

    let dashboard = dashboard_id(&id)?;
    let shares = DashboardShares::new(state.store(), &administrators, caller)
        .list(&dashboard)
        .await
        .map_err(sharing_error)?;

    let items: Vec<Value> = shares.iter().map(share_json).collect();
    Ok(Json(json!({ "items": items })).into_response())
}

async fn share_dashboard(
    Extension(state): Extension<Arc<AppState>>,
    Path((id, person)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let administrators = forwarded(&state, &headers);

    let dashboard = dashboard_id(&id)?;
    let person = person_id(&person, "person_id")?;
    DashboardShares::new(state.store(), &administrators, caller)
        .grant(&dashboard, person)
        .await
        .map_err(sharing_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn withdraw_share(
    Extension(state): Extension<Arc<AppState>>,
    Path((id, person)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let administrators = forwarded(&state, &headers);

    let dashboard = dashboard_id(&id)?;
    let person = person_id(&person, "person_id")?;
    DashboardShares::new(state.store(), &administrators, caller)
        .withdraw(&dashboard, person)
        .await
        .map_err(sharing_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list_orphaned(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(paging): Query<Paging>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let administrators = forwarded(&state, &headers);

    let page = Page::parse(paging.limit, paging.offset)
        .map_err(|error| DefinitionApiError::invalid_field("limit", error.to_string()))?;
    let found = DashboardShares::new(state.store(), &administrators, caller)
        .orphaned(page)
        .await
        .map_err(sharing_error)?;

    let items: Vec<Value> = found.items.iter().map(orphan_json).collect();
    Ok(Json(json!({
        "items": items,
        "total": found.total,
        "limit": page.limit(),
        "offset": page.offset(),
    }))
    .into_response())
}

async fn transfer_dashboard(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<TransferRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let Json(request) = body.map_err(|error| DefinitionApiError::unreadable_body(&error))?;
    let administrators = forwarded(&state, &headers);

    let dashboard = dashboard_id(&id)?;
    let to = person_id(&request.to, "to")?;
    DashboardShares::new(state.store(), &administrators, caller)
        .transfer(&dashboard, to)
        .await
        .map_err(sharing_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

fn forwarded<'a>(
    state: &'a AppState,
    headers: &'a axum::http::HeaderMap,
) -> ForwardedAdministrators<'a> {
    ForwardedAdministrators {
        identity: state.identity(),
        authorization: crate::api::forwarded_authorization(headers),
    }
}

#[cfg(test)]
mod tests;
