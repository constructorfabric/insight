//! Metric, widget and dashboard definition HTTP endpoints.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, Query, RawQuery};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::SecondsFormat;
use serde::{Deserialize, Serialize};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, Http, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::errors::ApiErrors;
use super::folders::{folder_error, holding_json};
use super::tags::{names_json, tag_error, tags_in};
use crate::domain::access::{Caller, DASHBOARD_NOT_EDITOR, DASHBOARD_NOT_OWNER, Owner, PersonId};
use crate::domain::dashboards::{DashboardQuery, ListedDashboard, QueryError};
use crate::domain::definition::{
    DefinitionId, DefinitionKind, DefinitionLabel, DefinitionStoreError, Listed, MAX_PAGE_LIMIT,
    Page, PageError, StoredDefinition,
};
use crate::domain::kinds::metric::answerable::EffectiveClock;
use crate::domain::surfaces::{CustomError, Surfaces};
use crate::domain::violation::Violation;

/// How long a listing waits for its owners' names before answering without.
const OWNER_NAMES_TIMEOUT: Duration = Duration::from_secs(2);

/// The query string on a list: what to look for, in a name or in a body, and
/// which page of the matches to answer with.
#[derive(Debug, Default, Deserialize)]
struct Search {
    #[serde(default)]
    q: String,
    limit: Option<u64>,
    offset: Option<u64>,
    folder: Option<String>,
    owned: Option<String>,
    sort: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct DefinitionRequest {
    /// What a reader calls it: trimmed, 1 to 128 characters, and shared by
    /// as many definitions as like.
    name: String,
    /// The definition body.
    body: serde_json::Value,
}

impl toolkit::api::api_dto::RequestApiDto for DefinitionRequest {}

#[derive(Debug, Serialize)]
struct CreatedResponse {
    id: String,
}

#[resource_error("gts.cf.insight.insight_v3_core.definitions.v1~")]
pub(super) struct DefinitionApiError;

impl ApiErrors for DefinitionApiError {
    fn invalid_field(field: &str, detail: String) -> CanonicalError {
        Self::invalid_argument()
            .with_field_violation(field, detail, "INVALID")
            .create()
    }

    fn timed_out(detail: &str) -> CanonicalError {
        Self::deadline_exceeded(detail).create()
    }

    fn name_taken(name: &str) -> CanonicalError {
        Self::already_exists(format!("`{name}` is already taken"))
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

// `.anonymous()`: these routes trust the gateway to authenticate the
// `__Host-sid` session cookie before forwarding. Must stay off the network
// (see docker-compose.yml's loopback port binding).
pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    DefinitionKind::ALL
        .into_iter()
        .fold(router, |router, kind| {
            register_kind(router, openapi, state.clone(), kind)
        })
}

fn query_param(name: &str, param_type: &str, description: &str) -> ParamSpec {
    ParamSpec::query(name)
        .description(description)
        .param_type(param_type)
}

fn register_list(
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
    kind: DefinitionKind,
    segment: &str,
) -> Router {
    let builder = OperationBuilder::get(format!("/v1/{segment}"))
        .operation_id(format!("insight_v3_core.{segment}.list"))
        .summary("List definitions by id and name, or the ones matching ?q=")
        .anonymous()
        .exposed()
        .param(query_param(
            "q",
            "string",
            "Text to look for in a name or in a stored body",
        ))
        .param(query_param(
            "limit",
            "integer",
            &format!("Page size, 1 to {MAX_PAGE_LIMIT}"),
        ))
        .param(query_param("offset", "integer", "Definitions to skip"));
    let builder = if kind == DefinitionKind::Dashboard {
        builder
            .param(query_param(
                "folder",
                "string",
                "Only the dashboards in this folder id, or `unfiled` for those in none",
            ))
            .param(
                query_param(
                    "tag",
                    "string",
                    "Only the dashboards carrying any of these tags; repeat the key for each",
                )
                .array(true),
            )
            .param(query_param(
                "owned",
                "string",
                "`me` for the caller's own dashboards, `shared` for those shared with them; absent lists every visible one",
            ))
            .param(query_param(
                "sort",
                "string",
                "`name` (the default) orders by name, `recent` by last change, newest first",
            ))
    } else {
        builder
    };

    builder
        .json_response(
            StatusCode::OK,
            "One page of ids and names, dashboards with their owner and the caller's access, and how many match",
        )
        .error_400(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_definitions)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()))
        .layer(Extension(kind))
}

fn register_kind(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
    kind: DefinitionKind,
) -> Router {
    let segment = kind.plural();

    let id_param = ParamSpec::path("id").description("Definition id");

    let dependents = register_dependents(openapi, &state, kind, segment, id_param.clone());

    let create = OperationBuilder::post(format!("/v1/{segment}"))
        .operation_id(format!("insight_v3_core.{segment}.create"))
        .summary("Create a definition under a fresh id")
        .anonymous()
        .exposed()
        .json_request::<DefinitionRequest>(openapi, "The definition's name and body")
        .json_response(StatusCode::CREATED, "The new definition's id")
        .error_400(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(create_definition)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()))
        .layer(Extension(kind));

    let replace = OperationBuilder::put(format!("/v1/{segment}/{{id}}"))
        .operation_id(format!("insight_v3_core.{segment}.put"))
        .summary("Replace an existing definition's name and body")
        .anonymous()
        .exposed()
        .param(id_param.clone())
        .json_request::<DefinitionRequest>(openapi, "The definition's name and body")
        .no_content_response(StatusCode::NO_CONTENT, "Definition stored")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(replace_definition)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()))
        .layer(Extension(kind));

    let get = OperationBuilder::get(format!("/v1/{segment}/{{id}}"))
        .operation_id(format!("insight_v3_core.{segment}.get"))
        .summary("Read a definition")
        .anonymous()
        .exposed()
        .param(id_param.clone())
        .json_response(
            StatusCode::OK,
            "The definition's id, name and body; a dashboard's owner, the caller's access, whether an entry they add carries, the folders they see holding it, and its tags",
        )
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(get_definition)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()))
        .layer(Extension(kind));

    let list = register_list(openapi, &state, kind, segment);

    let remove = OperationBuilder::delete(format!("/v1/{segment}/{{id}}"))
        .operation_id(format!("insight_v3_core.{segment}.delete"))
        .summary("Remove a definition")
        .anonymous()
        .exposed()
        .param(id_param)
        .no_content_response(StatusCode::NO_CONTENT, "Definition removed")
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(delete_definition)
        .register(Router::new(), openapi)
        .layer(Extension(state))
        .layer(Extension(kind));

    host_router
        .merge(create)
        .merge(replace)
        .merge(get)
        .merge(list)
        .merge(remove)
        .merge(dependents)
}

/// A stored definition as a reader is given it.
///
/// The body is what was written; the clock is not in it, because a metric
/// over a dataset may inherit one, and a reader deciding whether to window a
/// card cannot tell from the body alone.
#[derive(Debug, Serialize)]
struct DefinitionResponse {
    id: String,
    name: String,
    body: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    clock: Option<EffectiveClock>,
    #[serde(flatten)]
    meta: Option<DashboardMeta>,
}

#[derive(Debug, Serialize)]
struct DashboardMeta {
    owner_person_id: Option<String>,
    owner_name: Option<String>,
    access: &'static str,
    /// Whether an entry the caller adds now carries a folder's audience.
    carries: bool,
    /// The folders the caller sees that hold it.
    folders: Vec<serde_json::Value>,
    tags: serde_json::Value,
    updated_at: String,
}

/// An owner as the wire carries it: the person's id, or null for none.
pub(crate) fn owner_json(owner: Owner) -> Option<String> {
    owner.person().map(|person| person.to_string())
}

fn owner_name_json(owner: Owner, names: &HashMap<PersonId, String>) -> Option<String> {
    owner
        .person()
        .and_then(|person| names.get(&person).cloned())
}

/// One identity read for every owner on the page. A listing without names is
/// still worth showing, so a slow or failed read answers none.
async fn owner_names(
    state: &AppState,
    authorization: Option<&str>,
    owners: impl Iterator<Item = Owner>,
) -> HashMap<PersonId, String> {
    let mut people: Vec<PersonId> = owners.filter_map(Owner::person).collect();
    people.sort_unstable();
    people.dedup();
    if people.is_empty() {
        return HashMap::new();
    }

    let read = tokio::time::timeout(
        OWNER_NAMES_TIMEOUT,
        state.identity().names(authorization, &people),
    )
    .await;
    match read {
        Ok(Ok(names)) => names,
        Ok(Err(error)) => {
            tracing::warn!(%error, "owner names could not be read from identity");
            HashMap::new()
        }
        Err(_) => {
            tracing::warn!("owner names were not read from identity in time");
            HashMap::new()
        }
    }
}

async fn dashboard_meta(
    state: &AppState,
    surfaces: &Surfaces<'_>,
    authorization: Option<&str>,
    id: &DefinitionId,
    stored: &StoredDefinition,
) -> Result<DashboardMeta, CanonicalError> {
    let seen = surfaces.seen(id).await.map_err(custom_error)?;
    let names = owner_names(state, authorization, std::iter::once(seen.owner)).await;
    let folders = state
        .folders()
        .folders_holding(id, surfaces.caller())
        .await
        .map_err(folder_error)?;
    let tags = state.tags().tags_of(id).await.map_err(tag_error)?;

    Ok(DashboardMeta {
        owner_person_id: owner_json(seen.owner),
        owner_name: owner_name_json(seen.owner, &names),
        access: seen.access.as_str(),
        carries: seen.entry.carries(),
        folders: folders.iter().map(holding_json).collect(),
        tags: names_json(&tags),
        updated_at: stored
            .updated_at
            .to_rfc3339_opts(SecondsFormat::Millis, true),
    })
}

pub(super) fn custom_error(error: CustomError) -> CanonicalError {
    let detail = error.to_string();

    match error {
        CustomError::NotFound { kind, id } => {
            DefinitionApiError::not_found(format!("{} `{id}` was not found", kind.singular()))
                .with_resource(&id)
                .create()
        }
        CustomError::NotEditor => DefinitionApiError::permission_denied()
            .with_reason(DASHBOARD_NOT_EDITOR)
            .create(),
        CustomError::NotOwner => DefinitionApiError::permission_denied()
            .with_reason(DASHBOARD_NOT_OWNER)
            .create(),
        CustomError::InUse { .. } => DefinitionApiError::failed_precondition()
            .with_precondition_violation("name", detail, "in_use")
            .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
            .create(),
        CustomError::Store(DefinitionStoreError::TagsFull) => {
            DefinitionApiError::failed_precondition()
                .with_precondition_violation("tags", detail, "too_many")
                .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
                .create()
        }
        CustomError::Widget(source) => widget_error(&source),
        CustomError::DatasetNotReady(named) => DefinitionApiError::invalid_field(
            "body",
            format!("no dataset named `{named}` is ready"),
        ),
        CustomError::Body(source) => DefinitionApiError::invalid_field("body", source.to_string()),
        CustomError::Range(source) => {
            DefinitionApiError::invalid_field("time_ranges", source.to_string())
        }
        CustomError::Compile(source) => {
            DefinitionApiError::invalid_field("body", source.to_string())
        }
        CustomError::Unanswerable(violations) => unanswerable(&violations),
        CustomError::Store(source) => DefinitionApiError::definition_store_error(source),
        CustomError::Datasets(source) => DefinitionApiError::dataset_store_error(source),
        CustomError::Run(source) => {
            tracing::error!(error = ?source, "metric query execution failed");
            CanonicalError::internal("metric query execution failed").create()
        }
    }
}

/// Every way the dataset cannot answer the metric, reported together.
///
/// A metric wrong in several places is answered once, with each place named
/// as the submitted body shapes it, so an editor can mark all of them.
fn unanswerable(violations: &[Violation]) -> CanonicalError {
    let Some((first, rest)) = violations.split_first() else {
        return DefinitionApiError::invalid_field(
            "body",
            "the dataset cannot answer this metric".to_owned(),
        );
    };

    let mut builder = DefinitionApiError::invalid_argument().with_field_violation(
        &first.field,
        first.detail.clone(),
        first.reason_code(),
    );
    for violation in rest {
        builder = builder.with_field_violation(
            &violation.field,
            violation.detail.clone(),
            violation.reason_code(),
        );
    }

    builder.create()
}

async fn delete_definition(
    Extension(state): Extension<Arc<AppState>>,
    Extension(kind): Extension<DefinitionKind>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;

    let Ok(id) = DefinitionId::parse(&id) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    match state.surfaces(caller).delete(kind, &id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT.into_response()),
        Err(CustomError::NotFound { .. }) => Ok(StatusCode::NOT_FOUND.into_response()),
        Err(other) => Err(custom_error(other)),
    }
}

fn denied() -> CanonicalError {
    DefinitionApiError::permission_denied()
        .with_reason(crate::api::ADMIN_ONLY)
        .create()
}

fn unidentified() -> CanonicalError {
    DefinitionApiError::permission_denied()
        .with_reason(crate::api::UNIDENTIFIED)
        .create()
}

/// Who may read this kind: anyone signed in reads the dashboards they see,
/// and only an administrator the library.
async fn reader(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    kind: DefinitionKind,
) -> Result<Caller, CanonicalError> {
    match kind {
        DefinitionKind::Dashboard => crate::api::caller(state, headers, unidentified).await,
        DefinitionKind::Metric | DefinitionKind::Widget => {
            crate::api::require_admin(state, headers, denied).await
        }
    }
}

/// Every definition that names this one, so a reader sees what a removal would
/// break before they ask for one.
fn register_dependents(
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
    kind: DefinitionKind,
    segment: &str,
    id_param: ParamSpec,
) -> Router {
    OperationBuilder::get(format!("/v1/{segment}/{{id}}/dependents"))
        .operation_id(format!("insight_v3_core.{segment}.dependents"))
        .summary("Every definition that names this one")
        .anonymous()
        .exposed()
        .param(id_param)
        .json_response(StatusCode::OK, "What would break if this were removed")
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(definition_dependents)
        .register(Router::new(), openapi)
        .layer(Extension(Arc::clone(state)))
        .layer(Extension(kind))
}

async fn create_definition(
    Extension(state): Extension<Arc<AppState>>,
    Extension(kind): Extension<DefinitionKind>,
    headers: axum::http::HeaderMap,
    body: Result<Json<DefinitionRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let Json(request) = body.map_err(|error| DefinitionApiError::unreadable_body(&error))?;

    let name =
        DefinitionLabel::parse(&request.name).map_err(DefinitionApiError::definition_error)?;
    let id = state
        .surfaces(caller)
        .create(kind, &name, &request.body)
        .await
        .map_err(custom_error)?;

    let created = CreatedResponse {
        id: id.into_string(),
    };
    Ok((StatusCode::CREATED, Json(created)).into_response())
}

async fn replace_definition(
    Extension(state): Extension<Arc<AppState>>,
    Extension(kind): Extension<DefinitionKind>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<DefinitionRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;
    let Json(request) = body.map_err(|error| DefinitionApiError::unreadable_body(&error))?;

    let id = DefinitionId::parse(&id).map_err(|_| DefinitionApiError::malformed_id(kind, &id))?;
    let name =
        DefinitionLabel::parse(&request.name).map_err(DefinitionApiError::definition_error)?;
    state
        .surfaces(caller)
        .replace(kind, &id, &name, &request.body)
        .await
        .map_err(custom_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

fn widget_error(error: &crate::domain::kinds::widget::WidgetError) -> CanonicalError {
    DefinitionApiError::invalid_argument()
        .with_field_violation("body", error.to_string(), "INVALID")
        .create()
}

/// What names a definition, as a reader is shown it.
#[derive(Debug, Serialize)]
struct Dependents {
    /// Each holder the caller may see, as the kind it is, its id and the
    /// name it has.
    holders: Vec<Holder>,
    /// How many more holders are dashboards the caller may not see.
    hidden: u64,
}

#[derive(Debug, Serialize)]
struct Holder {
    kind: String,
    id: String,
    name: String,
}

/// Every definition that names this one, so a reader sees what a removal would
/// break before they ask for one.
async fn definition_dependents(
    Extension(state): Extension<Arc<AppState>>,
    Extension(kind): Extension<DefinitionKind>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;

    let Ok(id) = DefinitionId::parse(&id) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let surfaces = state.surfaces(caller);

    if let Err(CustomError::NotFound { .. }) = surfaces.get(kind, &id).await {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    let found = surfaces
        .dependents_of(kind, &id)
        .await
        .map_err(custom_error)?;
    let holders = found
        .holders
        .into_iter()
        .map(|holder| Holder {
            kind: holder.kind.plural().to_owned(),
            id: holder.id,
            name: holder.name,
        })
        .collect();

    Ok(Json(Dependents {
        holders,
        hidden: found.hidden,
    })
    .into_response())
}

async fn get_definition(
    Extension(state): Extension<Arc<AppState>>,
    Extension(kind): Extension<DefinitionKind>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = reader(&state, &headers, kind).await?;

    let Ok(id) = DefinitionId::parse(&id) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let surfaces = state.surfaces(caller);
    let stored = match surfaces.read(kind, &id).await {
        Ok(stored) => stored,
        Err(CustomError::NotFound { .. }) => return Ok(StatusCode::NOT_FOUND.into_response()),
        Err(other) => return Err(custom_error(other)),
    };

    let clock = surfaces.clock_of(kind, &stored.body).await;
    let meta = if kind == DefinitionKind::Dashboard {
        let authorization = crate::api::forwarded_authorization(&headers);
        Some(dashboard_meta(&state, &surfaces, authorization, &id, &stored).await?)
    } else {
        None
    };

    Ok(Json(DefinitionResponse {
        id: id.into_string(),
        name: stored.name,
        body: stored.body,
        clock,
        meta,
    })
    .into_response())
}

/// What a listing filter that cannot be read says, named by its parameter.
pub(crate) fn query_error(error: &QueryError) -> CanonicalError {
    let field = match error {
        QueryError::Folder(_) => "folder",
        QueryError::Tag(_) => "tag",
        QueryError::Owned(_) => "owned",
        QueryError::Order(_) => "sort",
    };

    DefinitionApiError::invalid_field(field, error.to_string())
}

/// One page of what the caller may list of this kind, as the wire shows it.
async fn listed_page(
    state: &AppState,
    surfaces: &Surfaces<'_>,
    authorization: Option<&str>,
    kind: DefinitionKind,
    query: &DashboardQuery,
    page: Page,
) -> Result<(serde_json::Value, u64), CustomError> {
    let Some(library) = kind.library() else {
        let found = surfaces.page_dashboards(query, page).await?;
        let names = owner_names(
            state,
            authorization,
            found.items.iter().map(|item| item.owner),
        )
        .await;
        return Ok((dashboards_json(&found.items, Some(&names)), found.total));
    };

    let found = surfaces.page(library, &query.needle, page).await?;

    Ok((listed_json(&found.items), found.total))
}

async fn list_definitions(
    Extension(state): Extension<Arc<AppState>>,
    Extension(kind): Extension<DefinitionKind>,
    headers: axum::http::HeaderMap,
    Query(search): Query<Search>,
    RawQuery(query): RawQuery,
) -> Result<Response, CanonicalError> {
    let caller = reader(&state, &headers, kind).await?;

    let page = Page::parse(search.limit, search.offset).map_err(page_error)?;
    let tags = tags_in(query.as_deref());
    let query = DashboardQuery::parse(
        kind,
        &search.q,
        search.folder.as_deref(),
        &tags,
        search.owned.as_deref(),
        search.sort.as_deref(),
    )
    .map_err(|error| query_error(&error))?;
    let authorization = crate::api::forwarded_authorization(&headers);
    let (items, total) = listed_page(
        &state,
        &state.surfaces(caller),
        authorization,
        kind,
        &query,
        page,
    )
    .await
    .map_err(custom_error)?;

    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "limit": page.limit(),
        "offset": page.offset(),
    }))
    .into_response())
}

pub(crate) fn listed_json(items: &[Listed]) -> serde_json::Value {
    serde_json::Value::Array(
        items
            .iter()
            .map(|listed| serde_json::json!({ "id": listed.id, "name": listed.name }))
            .collect(),
    )
}

/// Dashboards as a listing shows them: with their owner, how the caller sees
/// each, when it last changed and who its grants reach.
pub(crate) fn dashboards_json(
    items: &[ListedDashboard],
    names: Option<&HashMap<PersonId, String>>,
) -> serde_json::Value {
    serde_json::Value::Array(
        items
            .iter()
            .map(|listed| {
                let mut item = serde_json::json!({
                    "id": listed.id,
                    "name": listed.name,
                    "owner_person_id": owner_json(listed.owner),
                    "access": listed.access.as_str(),
                    "updated_at": listed
                        .updated_at
                        .to_rfc3339_opts(SecondsFormat::Millis, true),
                    "visibility": listed.visibility.as_str(),
                    "title": listed.title,
                    "tags": listed.tags,
                });
                if let Some(names) = names {
                    item["owner_name"] = owner_name_json(listed.owner, names).into();
                }
                item
            })
            .collect(),
    )
}

fn page_error(error: PageError) -> CanonicalError {
    DefinitionApiError::invalid_field("limit", error.to_string())
}

#[cfg(test)]
mod tests;
