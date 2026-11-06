mod body;
mod error;
mod routes;

use std::sync::Arc;

use axum::extract::{Extension, Path, rejection::JsonRejection};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use toolkit::api::OpenApiRegistry;
use toolkit_security::SecurityContext;

use super::{AppState, require_admin};
use crate::domain::connector_schedules::{self as domain, Change, Instance, Revision};
use domain::dto::ScheduleResponse;
use domain::validation::Crons;
use error::{ScheduleError, admin_only, domain_error};

pub(super) fn register(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    routes::register(router, openapi)
}

async fn get(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path((connector, source_id)): Path<(String, String)>,
) -> Result<Response, ScheduleError> {
    require_admin(&state, &headers, admin_only).await?;
    let instance = Instance::parse(&connector, &source_id).map_err(domain_error)?;
    let response = domain::get(&state.db, &state.ch, &instance)
        .await
        .map_err(domain_error)?;
    Ok(reply(response))
}

async fn put(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Path((connector, source_id)): Path<(String, String)>,
    request: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Response, ScheduleError> {
    require_admin(&state, &headers, admin_only).await?;
    let instance = Instance::parse(&connector, &source_id).map_err(domain_error)?;
    let editor = domain::editor(&state.ch, &instance)
        .await
        .map_err(domain_error)?;
    let revision = if_match(&headers).map_err(domain_error)?;
    let request = body::parse(request)?;
    let crons = Crons::parse(&request.crons)
        .map_err(|problem| domain_error(domain::Error::Invalid(problem)))?;
    let response = editor
        .write(&state.db, revision, Change::Put(crons), ctx.subject_id())
        .await
        .map_err(domain_error)?;
    Ok(reply(response))
}

async fn delete(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Path((connector, source_id)): Path<(String, String)>,
) -> Result<Response, ScheduleError> {
    require_admin(&state, &headers, admin_only).await?;
    let instance = Instance::parse(&connector, &source_id).map_err(domain_error)?;
    let editor = domain::clear_editor(&state.db, &state.ch, &instance)
        .await
        .map_err(domain_error)?;
    let revision = if_match(&headers).map_err(domain_error)?;
    let response = editor
        .write(&state.db, revision, Change::Delete, ctx.subject_id())
        .await
        .map_err(domain_error)?;
    Ok(reply(response))
}

fn if_match(headers: &HeaderMap) -> Result<Revision, domain::Error> {
    let mut values = headers.get_all(header::IF_MATCH).iter();
    let raw = values
        .next()
        .map(|value| value.to_str().map_err(|_| domain::Error::InvalidMatch))
        .transpose()?;
    if values.next().is_some() {
        return Err(domain::Error::InvalidMatch);
    }
    Revision::parse(raw)
}

fn reply(response: ScheduleResponse) -> Response {
    (
        [(header::ETAG, format!("\"{}\"", response.revision))],
        Json(response),
    )
        .into_response()
}

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod support;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod timeline_tests;

#[cfg(test)]
mod removed_tests;
