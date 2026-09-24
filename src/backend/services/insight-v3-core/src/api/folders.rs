use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamLocation, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, Http, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::errors::ApiErrors;
use crate::domain::definition::DefinitionName;
use crate::domain::folders::{Folder, FolderError, FolderId, FolderList, FolderName};

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
}

#[derive(Debug, Deserialize, ToSchema)]
struct NameRequest {
    /// Trimmed, then 1 to 64 characters, unique ignoring case.
    name: String,
}

impl toolkit::api::api_dto::RequestApiDto for NameRequest {}

#[derive(Debug, Deserialize, ToSchema)]
struct MoveRequest {
    /// The folder's id, or null for none.
    #[serde(deserialize_with = "Option::deserialize")]
    folder: Option<String>,
}

impl toolkit::api::api_dto::RequestApiDto for MoveRequest {}

pub(crate) fn folder_json(folder: &Folder) -> Value {
    json!({ "id": folder.id.to_string(), "name": folder.name.as_str() })
}

pub(crate) fn folders_json(list: &FolderList) -> Value {
    let folders: Vec<Value> = list
        .folders
        .iter()
        .map(|summary| {
            json!({
                "id": summary.folder.id.to_string(),
                "name": summary.folder.name.as_str(),
                "dashboards": summary.dashboards,
            })
        })
        .collect();

    json!({ "folders": folders, "unfiled": list.unfiled })
}

pub(super) fn folder_error(error: FolderError) -> CanonicalError {
    let detail = error.to_string();

    match error {
        FolderError::Name => FolderApiError::invalid_field("name", detail),
        FolderError::NotFiled(_) => FolderApiError::invalid_field("folder", detail),
        FolderError::Id => FolderApiError::invalid_field("id", detail),
        FolderError::FolderNotFound(id) => FolderApiError::not_found(detail)
            .with_resource(id.to_string())
            .create(),
        FolderError::DashboardNotFound(name) => FolderApiError::not_found(detail)
            .with_resource(name)
            .create(),
        FolderError::NameTaken(name) => FolderApiError::name_taken(&name),
        FolderError::TooMany => FolderApiError::failed_precondition()
            .with_precondition_violation("name", detail, "too_many")
            .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
            .create(),
        FolderError::Store(source) => FolderApiError::definition_store_error(source),
    }
}

pub(super) fn folder_field_error(error: &FolderError) -> CanonicalError {
    FolderApiError::invalid_field("folder", error.to_string())
}

fn denied() -> CanonicalError {
    FolderApiError::permission_denied()
        .with_reason(crate::api::ADMIN_ONLY)
        .create()
}

fn path_param(name: &str, description: &str) -> ParamSpec {
    ParamSpec {
        name: name.to_owned(),
        location: ParamLocation::Path,
        required: true,
        description: Some(description.to_owned()),
        param_type: "string".to_owned(),
        array: false,
    }
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    let id_param = path_param("id", "Folder id");

    let list = OperationBuilder::get("/v1/folders")
        .operation_id("insight_v3_core.folders.list")
        .summary("List the dashboard folders, each with how many dashboards it holds")
        .anonymous()
        .exposed()
        .json_response(
            StatusCode::OK,
            "Every folder, and how many dashboards are in none",
        )
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_folders)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let create = OperationBuilder::post("/v1/folders")
        .operation_id("insight_v3_core.folders.create")
        .summary("Make a dashboard folder")
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
        .summary("Rename a dashboard folder")
        .anonymous()
        .exposed()
        .param(id_param.clone())
        .json_request::<NameRequest>(openapi, "The folder's new name")
        .json_response(StatusCode::OK, "The folder under its new name")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(rename_folder)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let remove = OperationBuilder::delete("/v1/folders/{id}")
        .operation_id("insight_v3_core.folders.delete")
        .summary("Remove a dashboard folder; its dashboards are left in none")
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

    let file = OperationBuilder::put("/v1/dashboards/{name}/folder")
        .operation_id("insight_v3_core.dashboards.folder")
        .summary("Move a dashboard into a folder, or out of every folder")
        .anonymous()
        .exposed()
        .param(path_param("name", "Dashboard name"))
        .json_request::<MoveRequest>(openapi, "The folder to hold it")
        .no_content_response(StatusCode::NO_CONTENT, "Dashboard moved")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(file_dashboard)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    router
        .merge(list)
        .merge(create)
        .merge(rename)
        .merge(remove)
        .merge(file)
}

async fn list_folders(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;

    let listed = state.folders().list_folders().await.map_err(folder_error)?;

    Ok(Json(folders_json(&listed)).into_response())
}

async fn create_folder(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: Result<Json<NameRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;
    let Json(request) = body.map_err(|error| FolderApiError::unreadable_body(&error))?;

    let name = FolderName::parse(&request.name).map_err(folder_error)?;
    let folder = state
        .folders()
        .create_folder(name)
        .await
        .map_err(folder_error)?;

    Ok((StatusCode::CREATED, Json(folder_json(&folder))).into_response())
}

async fn rename_folder(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<NameRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;
    let Json(request) = body.map_err(|error| FolderApiError::unreadable_body(&error))?;

    let id = FolderId::parse(&id).map_err(folder_error)?;
    let name = FolderName::parse(&request.name).map_err(folder_error)?;
    let folder = state
        .folders()
        .rename_folder(id, name)
        .await
        .map_err(folder_error)?;

    Ok(Json(folder_json(&folder)).into_response())
}

async fn delete_folder(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;

    let id = FolderId::parse(&id).map_err(folder_error)?;
    let removed = state
        .folders()
        .delete_folder(id)
        .await
        .map_err(folder_error)?;
    if !removed {
        return Err(folder_error(FolderError::FolderNotFound(id)));
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn file_dashboard(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<MoveRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;
    let Json(request) = body.map_err(|error| FolderApiError::unreadable_body(&error))?;

    let dashboard = DefinitionName::parse(&name).map_err(FolderApiError::definition_error)?;
    let folder = request
        .folder
        .as_deref()
        .map(FolderId::parse)
        .transpose()
        .map_err(|error| folder_field_error(&error))?;
    state
        .folders()
        .file(&dashboard, folder)
        .await
        .map_err(folder_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests;
