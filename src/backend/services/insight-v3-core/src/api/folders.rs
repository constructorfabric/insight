use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, Http, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::definitions::owner_json;
use super::errors::ApiErrors;
use crate::domain::access::{FOLDER_NOT_EDITOR, FOLDER_NOT_OWNER, FolderAccess};
use crate::domain::definition::{DefinitionId, DefinitionKind};
use crate::domain::folders::{
    self, Folder, FolderError, FolderId, FolderList, FolderName, SeenFolder,
};

#[resource_error("gts.cf.insight.insight_v3_core.folders.v1~")]
struct FolderApiError;

impl ApiErrors for FolderApiError {
    fn invalid_field(field: &str, detail: String) -> CanonicalError {
        Self::invalid_argument()
            .with_field_violation(field, detail, "INVALID")
            .create()
    }

    fn timed_out(detail: &str) -> CanonicalError {
        Self::deadline_exceeded(detail).create()
    }

    fn name_taken(name: &str) -> CanonicalError {
        Self::already_exists(format!("a folder named `{name}` already exists"))
            .with_resource(name)
            .create()
    }

    fn missing(resource: &str, detail: String) -> CanonicalError {
        Self::not_found(detail).with_resource(resource).create()
    }

    fn oversized(field: &str, detail: &str) -> CanonicalError {
        Self::invalid_argument()
            .with_field_violation(field, detail, "TOO_LARGE")
            .create()
    }
}

#[derive(Debug, Deserialize, ToSchema)]
struct NameRequest {
    /// Trimmed, then 1 to 64 characters.
    name: String,
}

impl toolkit::api::api_dto::RequestApiDto for NameRequest {}

/// A folder as a dashboard's reader is told it holds the dashboard.
pub(crate) fn holding_json(folder: &Folder) -> Value {
    json!({
        "id": folder.id.to_string(),
        "name": folder.name.as_str(),
    })
}

pub(crate) fn folder_json(seen: &SeenFolder) -> Value {
    json!({
        "id": seen.folder.id.to_string(),
        "name": seen.folder.name.as_str(),
        "owner_person_id": owner_json(seen.folder.owner),
        "access": seen.access.as_str(),
    })
}

pub(crate) fn folders_json(list: &FolderList) -> Value {
    let folders: Vec<Value> = list
        .folders
        .iter()
        .map(|summary| {
            let mut folder = folder_json(&summary.seen);
            folder["dashboards"] = json!(summary.dashboards);
            folder
        })
        .collect();

    json!({ "folders": folders, "unfiled": list.unfiled })
}

pub(crate) fn folder_error(error: FolderError) -> CanonicalError {
    let detail = error.to_string();

    match error {
        FolderError::Name => FolderApiError::invalid_field("name", detail),
        FolderError::NotFiled(_) => FolderApiError::invalid_field("folder", detail),
        FolderError::WrongShelf(_) => FolderApiError::failed_precondition()
            .with_precondition_violation("folder", detail, "wrong_shelf")
            .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
            .create(),
        FolderError::Id => FolderApiError::invalid_field("id", detail),
        FolderError::FolderNotFound(id) => FolderApiError::not_found(detail)
            .with_resource(id.to_string())
            .create(),
        FolderError::DashboardNotFound(name) => FolderApiError::not_found(detail)
            .with_resource(name)
            .create(),
        FolderError::TooMany => FolderApiError::failed_precondition()
            .with_precondition_violation("name", detail, "too_many")
            .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
            .create(),
        FolderError::NotEditor => FolderApiError::permission_denied()
            .with_reason(FOLDER_NOT_EDITOR)
            .create(),
        FolderError::NotOwner => FolderApiError::permission_denied()
            .with_reason(FOLDER_NOT_OWNER)
            .create(),
        FolderError::Store(source) => FolderApiError::definition_store_error(source),
    }
}

fn unidentified() -> CanonicalError {
    FolderApiError::permission_denied()
        .with_reason(crate::api::UNIDENTIFIED)
        .create()
}

fn path_param(name: &str, description: &str) -> ParamSpec {
    ParamSpec::path(name).description(description)
}

/// A folder id in a path; one that cannot be an id names nothing.
fn folder_id(raw: &str) -> Result<FolderId, CanonicalError> {
    FolderId::parse(raw).map_err(|_| folder_error(FolderError::Id))
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    let id_param = path_param("id", "Folder id");

    let list = OperationBuilder::get("/v1/folders")
        .operation_id("insight_v3_core.folders.list")
        .summary("List the folders the caller sees, each with their access and how many dashboards they see in it")
        .anonymous()
        .exposed()
        .json_response(
            StatusCode::OK,
            "The folders, and how many visible dashboards are in none of them",
        )
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_folders)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let create = OperationBuilder::post("/v1/folders")
        .operation_id("insight_v3_core.folders.create")
        .summary("Make a folder of the caller's own")
        .anonymous()
        .exposed()
        .json_request::<NameRequest>(openapi, "The folder's name")
        .json_response(StatusCode::CREATED, "The folder, with its id")
        .error_400(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(create_folder)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let rename = OperationBuilder::patch("/v1/folders/{id}")
        .operation_id("insight_v3_core.folders.rename")
        .summary("Rename a folder the caller owns or edits, or an ownerless one")
        .anonymous()
        .exposed()
        .param(id_param.clone())
        .json_request::<NameRequest>(openapi, "The folder's new name")
        .json_response(StatusCode::OK, "The folder under its new name")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(rename_folder)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let remove = OperationBuilder::delete("/v1/folders/{id}")
        .operation_id("insight_v3_core.folders.delete")
        .summary("Remove a folder the caller owns, or an ownerless one; its dashboards stay")
        .anonymous()
        .exposed()
        .param(id_param)
        .no_content_response(StatusCode::NO_CONTENT, "Folder removed")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(delete_folder)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    router
        .merge(list)
        .merge(create)
        .merge(rename)
        .merge(remove)
        .merge(entry_routes(openapi, state))
}

fn entry_routes(openapi: &dyn OpenApiRegistry, state: &Arc<AppState>) -> Router {
    let folder = || path_param("id", "Folder id");
    let dashboard = || path_param("dashboard_id", "Dashboard id");

    let add = OperationBuilder::put("/v1/folders/{id}/dashboards/{dashboard_id}")
        .operation_id("insight_v3_core.folders.dashboards.put")
        .summary("Put a dashboard the caller sees in a folder they own or edit; it carries the folder's audience only where they own the dashboard or hold an editor grant on it")
        .anonymous()
        .exposed()
        .param(folder())
        .param(dashboard())
        .json_response(
            StatusCode::OK,
            "Whether the entry carries the folder's audience to the dashboard",
        )
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(add_entry)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let remove = OperationBuilder::delete("/v1/folders/{id}/dashboards/{dashboard_id}")
        .operation_id("insight_v3_core.folders.dashboards.delete")
        .summary("Take a dashboard out of a folder the caller owns or edits; the dashboard stays")
        .anonymous()
        .exposed()
        .param(folder())
        .param(dashboard())
        .no_content_response(StatusCode::NO_CONTENT, "Not in the folder")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(remove_entry)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    Router::new().merge(add).merge(remove)
}

async fn list_folders(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::caller(&state, &headers, unidentified).await?;

    let listed = state
        .folders()
        .list_folders(caller)
        .await
        .map_err(folder_error)?;

    Ok(Json(folders_json(&listed)).into_response())
}

async fn create_folder(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: Result<Json<NameRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::caller(&state, &headers, unidentified).await?;
    let Json(request) = body.map_err(|error| FolderApiError::unreadable_body(&error))?;

    let name = FolderName::parse(&request.name).map_err(folder_error)?;
    let folder = state
        .folders()
        .create_folder(caller.person, name)
        .await
        .map_err(folder_error)?;
    let seen = SeenFolder {
        folder,
        access: FolderAccess::Owner,
    };

    Ok((StatusCode::CREATED, Json(folder_json(&seen))).into_response())
}

async fn rename_folder(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<NameRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::caller(&state, &headers, unidentified).await?;
    let Json(request) = body.map_err(|error| FolderApiError::unreadable_body(&error))?;

    let id = folder_id(&id)?;
    let name = FolderName::parse(&request.name).map_err(folder_error)?;
    let seen = folders::rename(state.store(), caller, id, name)
        .await
        .map_err(folder_error)?;

    Ok(Json(folder_json(&seen)).into_response())
}

async fn delete_folder(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::caller(&state, &headers, unidentified).await?;

    let id = folder_id(&id)?;
    folders::delete(state.store(), caller, id)
        .await
        .map_err(folder_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

fn dashboard_id(raw: &str) -> Result<DefinitionId, CanonicalError> {
    DefinitionId::parse(raw)
        .map_err(|_| FolderApiError::malformed_id(DefinitionKind::Dashboard, raw))
}

async fn add_entry(
    Extension(state): Extension<Arc<AppState>>,
    Path((id, dashboard)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::caller(&state, &headers, unidentified).await?;

    let folder = folder_id(&id)?;
    let dashboard = dashboard_id(&dashboard)?;
    let entry = folders::add(state.store(), caller, folder, &dashboard)
        .await
        .map_err(folder_error)?;

    Ok(Json(json!({ "carries": entry.carries() })).into_response())
}

async fn remove_entry(
    Extension(state): Extension<Arc<AppState>>,
    Path((id, dashboard)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::caller(&state, &headers, unidentified).await?;

    let folder = folder_id(&id)?;
    let dashboard = dashboard_id(&dashboard)?;
    folders::remove(state.store(), caller, folder, &dashboard)
        .await
        .map_err(folder_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests;
