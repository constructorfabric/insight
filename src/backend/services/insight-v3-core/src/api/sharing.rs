//! Granting dashboards and folders, and handing an orphaned one to a current
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
use super::definitions::DefinitionApiError;
use super::errors::ApiErrors;
use crate::domain::access::{DASHBOARD_NOT_EDITOR, FOLDER_NOT_EDITOR, Grantee, Level, PersonId};
use crate::domain::definition::{DefinitionId, DefinitionKind, MAX_PAGE_LIMIT, Page};
use crate::domain::folders::FolderId;
use crate::domain::sharing::{
    AdministratorsError, Directory, Grant, Orphan, OrphanPage, Shared, Sharing, SharingError,
};
use crate::store::identity::{IdentityClient, IdentityError};

#[derive(Debug, Deserialize, ToSchema)]
struct TransferRequest {
    /// The person id of the current administrator to own it from now on.
    to: String,
}

impl toolkit::api::api_dto::RequestApiDto for TransferRequest {}

#[derive(Debug, Deserialize, ToSchema)]
struct GrantRequest {
    /// `viewer` or `editor`. An editor who is not an administrator only
    /// views.
    level: String,
}

impl toolkit::api::api_dto::RequestApiDto for GrantRequest {}

#[derive(Debug, Default, Deserialize)]
struct Paging {
    limit: Option<u64>,
    offset: Option<u64>,
}

/// Identity, asked with the caller's own authorization.
struct ForwardedDirectory<'a> {
    identity: &'a IdentityClient,
    authorization: Option<&'a str>,
}

fn unavailable(error: &IdentityError) -> AdministratorsError {
    AdministratorsError::Unavailable(error.to_string())
}

#[async_trait]
impl Directory for ForwardedDirectory<'_> {
    async fn holds_admin(&self, person: PersonId) -> Result<bool, AdministratorsError> {
        self.identity
            .holds_admin(self.authorization, person)
            .await
            .map_err(|error| unavailable(&error))
    }

    async fn current(&self) -> Result<Vec<PersonId>, AdministratorsError> {
        self.identity
            .admins(self.authorization)
            .await
            .map_err(|error| match error {
                IdentityError::Truncated => AdministratorsError::Truncated,
                IdentityError::Transport(_) | IdentityError::Refused(_) => unavailable(&error),
            })
    }

    async fn knows(&self, person: PersonId) -> Result<bool, AdministratorsError> {
        self.identity
            .knows(self.authorization, person)
            .await
            .map_err(|error| unavailable(&error))
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
        SharingError::FolderNotFound(id) => DefinitionApiError::not_found(detail)
            .with_resource(id.to_string())
            .create(),
        SharingError::NotEditor => DefinitionApiError::permission_denied()
            .with_reason(DASHBOARD_NOT_EDITOR)
            .create(),
        SharingError::FolderNotEditor => DefinitionApiError::permission_denied()
            .with_reason(FOLDER_NOT_EDITOR)
            .create(),
        SharingError::Ownerless(_) => conflict("owner", detail, "ownerless"),
        SharingError::OwnerIsAdministrator(_) => {
            conflict("owner", detail, "owner_is_administrator")
        }
        SharingError::FolderUnshared => conflict("grants", detail, "not_shared"),
        SharingError::Moved(_) => conflict("owner", detail, "owner_changed"),
        SharingError::RecipientTagsFull => conflict("to", detail, "too_many_tags"),
        SharingError::GrantToOwner | SharingError::UnknownPerson(_) => {
            DefinitionApiError::invalid_field("grantee", detail)
        }
        SharingError::RecipientNotAdministrator(_) => {
            DefinitionApiError::invalid_field("to", detail)
        }
        SharingError::Administrators(source) => {
            tracing::error!(error = %source, "identity could not be read");
            CanonicalError::internal("failed to verify people").create()
        }
        SharingError::Store(source) => DefinitionApiError::definition_store_error(source),
    }
}

fn dashboard_id(raw: &str) -> Result<DefinitionId, CanonicalError> {
    DefinitionId::parse(raw)
        .map_err(|_| DefinitionApiError::malformed_id(DefinitionKind::Dashboard, raw))
}

fn folder_id(raw: &str) -> Result<FolderId, CanonicalError> {
    FolderId::parse(raw).map_err(|_| {
        DefinitionApiError::not_found(format!("no folder has the id `{raw}`"))
            .with_resource(raw)
            .create()
    })
}

fn grantee(raw: &str) -> Result<Grantee, CanonicalError> {
    Grantee::parse(raw).ok_or_else(|| {
        DefinitionApiError::invalid_field(
            "grantee",
            "a grantee is a person id or `everyone`".to_owned(),
        )
    })
}

fn level(raw: &str) -> Result<Level, CanonicalError> {
    Level::parse(raw).ok_or_else(|| {
        DefinitionApiError::invalid_field("level", "a level is `viewer` or `editor`".to_owned())
    })
}

fn person_id(raw: &str, field: &str) -> Result<PersonId, CanonicalError> {
    PersonId::parse(raw)
        .ok_or_else(|| DefinitionApiError::invalid_field(field, "a person id is a UUID".to_owned()))
}

fn grant_json(grant: &Grant) -> Value {
    json!({
        "grantee": grant.grantee.to_string(),
        "level": grant.level.as_str(),
        "granted_by": grant.granted_by.to_string(),
        "created_at": grant.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
    })
}

fn orphans_json(found: &OrphanPage, page: Page) -> Value {
    let items: Vec<Value> = found.items.iter().map(orphan_json).collect();

    json!({
        "items": items,
        "total": found.total,
        "limit": page.limit(),
        "offset": page.offset(),
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
        .merge(grant_routes(openapi, state, Object::Dashboard))
        .merge(grant_routes(openapi, state, Object::Folder))
        .merge(orphan_routes(openapi, state))
        .merge(transfer_routes(openapi, state))
}

/// What a grant route is on.
#[derive(Debug, Clone, Copy)]
enum Object {
    Dashboard,
    Folder,
}

impl Object {
    fn segment(self) -> &'static str {
        match self {
            Self::Dashboard => "dashboards",
            Self::Folder => "folders",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Dashboard => "dashboard",
            Self::Folder => "folder",
        }
    }
}

/// The thing a route's `{id}` names, parsed as its object's id.
enum Named {
    Dashboard(DefinitionId),
    Folder(FolderId),
}

impl Named {
    fn parse(object: Object, raw: &str) -> Result<Self, CanonicalError> {
        match object {
            Object::Dashboard => dashboard_id(raw).map(Self::Dashboard),
            Object::Folder => folder_id(raw).map(Self::Folder),
        }
    }

    fn shared(&self) -> Shared<'_> {
        match self {
            Self::Dashboard(id) => Shared::Dashboard(id),
            Self::Folder(id) => Shared::Folder(*id),
        }
    }
}

fn grant_routes(openapi: &dyn OpenApiRegistry, state: &Arc<AppState>, object: Object) -> Router {
    let segment = object.segment();
    let noun = object.noun();
    let id = || path_param("id", &format!("The {noun}'s id"));
    let grantee = || path_param("grantee", "A person id, or `everyone`");

    let list = OperationBuilder::get(format!("/v1/{segment}/{{id}}/grants"))
        .operation_id(format!("insight_v3_core.{segment}.grants.list"))
        .summary(format!(
            "List the grants of a {noun} the caller owns or edits"
        ))
        .anonymous()
        .exposed()
        .param(id())
        .json_response(
            StatusCode::OK,
            "Each grant: who it reaches, its level, who granted it, and when",
        )
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_grants)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()))
        .layer(Extension(object));

    let grant = OperationBuilder::put(format!("/v1/{segment}/{{id}}/grants/{{grantee}}"))
        .operation_id(format!("insight_v3_core.{segment}.grants.put"))
        .summary(format!(
            "Grant a {noun} to a person or to everyone; granting again sets the level"
        ))
        .anonymous()
        .exposed()
        .param(id())
        .param(grantee())
        .json_request::<GrantRequest>(openapi, "The level to grant")
        .no_content_response(StatusCode::NO_CONTENT, "Granted")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(put_grant)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()))
        .layer(Extension(object));

    let withdraw = OperationBuilder::delete(format!("/v1/{segment}/{{id}}/grants/{{grantee}}"))
        .operation_id(format!("insight_v3_core.{segment}.grants.delete"))
        .summary(format!(
            "Withdraw a grant of a {noun}; pins and folder entries stay and show again if access returns"
        ))
        .anonymous()
        .exposed()
        .param(id())
        .param(grantee())
        .no_content_response(StatusCode::NO_CONTENT, "Not granted")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(withdraw_grant)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()))
        .layer(Extension(object));

    Router::new().merge(list).merge(grant).merge(withdraw)
}

fn orphan_routes(openapi: &dyn OpenApiRegistry, state: &Arc<AppState>) -> Router {
    let route = |object: Object, summary: &str| {
        let segment = object.segment();

        OperationBuilder::get(format!("/v1/orphaned-{segment}"))
            .operation_id(format!("insight_v3_core.orphaned_{segment}.list"))
            .summary(summary)
            .anonymous()
            .exposed()
            .param(
                ParamSpec::query("limit")
                    .description(format!("Page size, 1 to {MAX_PAGE_LIMIT}"))
                    .param_type("integer"),
            )
            .param(
                ParamSpec::query("offset")
                    .description(format!("{} to skip", object.noun()))
                    .param_type("integer"),
            )
            .json_response(StatusCode::OK, "One page of them, and how many there are")
            .error_400(openapi)
            .error_403(openapi)
            .error_500(openapi)
            .error_504(openapi)
            .handler(list_orphaned)
            .register(Router::new(), openapi)
            .layer(Extension(state.clone()))
            .layer(Extension(object))
    };

    Router::new()
        .merge(route(
            Object::Dashboard,
            "List the dashboards whose owner is no longer an administrator",
        ))
        .merge(route(
            Object::Folder,
            "List the folders shared with someone whose owner is no longer an administrator",
        ))
}

fn transfer_routes(openapi: &dyn OpenApiRegistry, state: &Arc<AppState>) -> Router {
    let route = |object: Object| {
        let segment = object.segment();
        let noun = object.noun();

        OperationBuilder::post(format!("/v1/{segment}/{{id}}/transfer"))
            .operation_id(format!("insight_v3_core.{segment}.transfer"))
            .summary(format!(
                "Hand an orphaned {noun} to a current administrator"
            ))
            .anonymous()
            .exposed()
            .param(path_param("id", &format!("The {noun}'s id")))
            .json_request::<TransferRequest>(openapi, "Who owns it from now on")
            .no_content_response(StatusCode::NO_CONTENT, "Transferred")
            .error_400(openapi)
            .error_403(openapi)
            .error_404(openapi)
            .error_409(openapi)
            .error_500(openapi)
            .error_504(openapi)
            .handler(transfer)
            .register(Router::new(), openapi)
            .layer(Extension(state.clone()))
            .layer(Extension(object))
    };

    Router::new()
        .merge(route(Object::Dashboard))
        .merge(route(Object::Folder))
}

async fn list_grants(
    Extension(state): Extension<Arc<AppState>>,
    Extension(object): Extension<Object>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let directory = forwarded(&state, &headers);

    let named = Named::parse(object, &id)?;
    let grants = Sharing::new(state.store(), &directory, caller)
        .list(named.shared())
        .await
        .map_err(sharing_error)?;

    let items: Vec<Value> = grants.iter().map(grant_json).collect();
    Ok(Json(json!({ "items": items })).into_response())
}

async fn put_grant(
    Extension(state): Extension<Arc<AppState>>,
    Extension(object): Extension<Object>,
    Path((id, raw_grantee)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    body: Result<Json<GrantRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let Json(request) = body.map_err(|error| DefinitionApiError::unreadable_body(&error))?;
    let directory = forwarded(&state, &headers);

    let named = Named::parse(object, &id)?;
    let grantee = grantee(&raw_grantee)?;
    let level = level(&request.level)?;
    Sharing::new(state.store(), &directory, caller)
        .grant(named.shared(), grantee, level)
        .await
        .map_err(sharing_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn withdraw_grant(
    Extension(state): Extension<Arc<AppState>>,
    Extension(object): Extension<Object>,
    Path((id, raw_grantee)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let directory = forwarded(&state, &headers);

    let named = Named::parse(object, &id)?;
    let grantee = grantee(&raw_grantee)?;
    Sharing::new(state.store(), &directory, caller)
        .withdraw(named.shared(), grantee)
        .await
        .map_err(sharing_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list_orphaned(
    Extension(state): Extension<Arc<AppState>>,
    Extension(object): Extension<Object>,
    headers: axum::http::HeaderMap,
    Query(paging): Query<Paging>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let directory = forwarded(&state, &headers);

    let page = Page::parse(paging.limit, paging.offset)
        .map_err(|error| DefinitionApiError::invalid_field("limit", error.to_string()))?;
    let sharing = Sharing::new(state.store(), &directory, caller);
    let found = match object {
        Object::Dashboard => sharing.orphaned(page).await,
        Object::Folder => sharing.orphaned_folders(page).await,
    }
    .map_err(sharing_error)?;

    Ok(Json(orphans_json(&found, page)).into_response())
}

async fn transfer(
    Extension(state): Extension<Arc<AppState>>,
    Extension(object): Extension<Object>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<TransferRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let Json(request) = body.map_err(|error| DefinitionApiError::unreadable_body(&error))?;
    let directory = forwarded(&state, &headers);

    let named = Named::parse(object, &id)?;
    let to = person_id(&request.to, "to")?;
    let sharing = Sharing::new(state.store(), &directory, caller);
    match &named {
        Named::Dashboard(dashboard) => sharing.transfer(dashboard, to).await,
        Named::Folder(folder) => sharing.transfer_folder(*folder, to).await,
    }
    .map_err(sharing_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

fn forwarded<'a>(
    state: &'a AppState,
    headers: &'a axum::http::HeaderMap,
) -> ForwardedDirectory<'a> {
    ForwardedDirectory {
        identity: state.identity(),
        authorization: crate::api::forwarded_authorization(headers),
    }
}

#[cfg(test)]
mod tests;
