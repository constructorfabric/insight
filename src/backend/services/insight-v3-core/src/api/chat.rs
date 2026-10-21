//! The chat endpoint: answers a question from the data, or writes
//! metric/widget/dashboard definitions.

use std::sync::Arc;

use axum::extract::Extension;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use toolkit::api::{OpenApiRegistry, OperationBuilder};
use toolkit_canonical_errors::{CanonicalError, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::errors::ApiErrors;
use crate::chat::{Ask, ChatError, Proposal, Proposed, Turn};
use crate::domain::access::PersonId;
use crate::domain::assistant::DatasetSchemas;
use crate::domain::definition::{
    Change, Definition, DefinitionId, DefinitionKind, DefinitionLabel,
};
use crate::domain::kinds;
use crate::domain::query::metric_query::{MetricQuery, RunResult};
use crate::domain::surfaces::{Surfaces, creating};

#[resource_error("gts.cf.insight.insight_v3_core.chat.v1~")]
struct ChatApiError;

impl ApiErrors for ChatApiError {
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

#[derive(Debug, Deserialize, ToSchema)]
struct ChatRequest {
    message: String,
    /// The turns before this one. The service keeps no session, so the panel
    /// sends the thread back; without it the model answered a follow-up with
    /// no idea what came before it.
    #[serde(default)]
    history: Vec<Turn>,
}
impl toolkit::api::api_dto::RequestApiDto for ChatRequest {}

#[derive(Debug, Serialize)]
struct ChatAnswerResponse {
    reply: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<RunResult>,
}

#[derive(Debug, Serialize)]
struct ChatCreatedResponse {
    reply: String,
    created: Written,
    /// What already existed under these ids and now holds something else.
    updated: Written,
}

#[derive(Debug, Default, Serialize)]
struct Written {
    metric: Option<WrittenOne>,
    widgets: Vec<WrittenOne>,
    dashboard: Option<WrittenOne>,
}

#[derive(Debug, Serialize)]
struct WrittenOne {
    id: String,
    name: String,
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let chat = OperationBuilder::post("/v1/chat")
        .operation_id("insight_v3_core.chat")
        .summary("Answer a question from the data, or write metric/widget/dashboard definitions")
        .anonymous()
        .exposed()
        .json_request::<ChatRequest>(openapi, "The chat message")
        .json_response(StatusCode::OK, "The model's reply")
        .error_400(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(handle_chat)
        .register(Router::new(), openapi)
        .layer(Extension(state));

    router.merge(chat)
}

async fn handle_chat(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    request: Result<Json<ChatRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, || {
        ChatApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await?;
    let Json(request) = request.map_err(|error| ChatApiError::unreadable_body(&error))?;

    let briefing = state.assistant().briefing(caller).await;
    let schemas = DatasetSchemas::new(state.datasets(), state.store());
    let proposal = state
        .chat()
        .propose(&Ask {
            message: &request.message,
            turns: &request.history,
            datasets: &briefing.datasets,
            catalogue: &briefing.catalogue,
            allowed: &briefing.allowed,
            schemas: &schemas,
            people: state.metrics().people(),
        })
        .await
        .map_err(chat_error)?;

    match proposal {
        Proposal::Answer { reply, query } => {
            let queried = query
                .as_ref()
                .and_then(MetricQuery::dataset)
                .map(str::to_owned);
            let result = answered_with(&state, query).await?;
            let reply = match &result {
                Some(rows) if rows.rows.is_empty() => no_rows(queried.as_deref()),
                _ => reply,
            };

            Ok(Json(ChatAnswerResponse { reply, result }).into_response())
        }
        Proposal::Create {
            reply,
            metric,
            widgets,
            dashboard,
        } => store_proposed(&state, caller, reply, metric, widgets, dashboard).await,
    }
}

/// One proposed item, once its name and id are known good.
#[derive(Debug)]
struct Landing {
    kind: DefinitionKind,
    id: DefinitionId,
    /// Also what the other items of the proposal call it by.
    name: DefinitionLabel,
    replaces: bool,
    body: serde_json::Value,
}

/// Stores what the assistant proposed, as `caller`, or none of it.
///
/// Every name and id is parsed and every body checked before anything is
/// written, so one refusal leaves the reader what they had rather than half a
/// dashboard. A dashboard it creates is the caller's; one it replaces must be
/// one the caller may change.
async fn store_proposed(
    state: &AppState,
    caller: PersonId,
    reply: String,
    metric: Option<Proposed>,
    widgets: Vec<Proposed>,
    dashboard: Option<Proposed>,
) -> Result<Response, CanonicalError> {
    let mut asked = Vec::new();
    if let Some(proposed) = metric {
        asked.push((DefinitionKind::Metric, proposed));
    }
    for proposed in widgets {
        asked.push((DefinitionKind::Widget, proposed));
    }
    if let Some(proposed) = dashboard {
        asked.push((DefinitionKind::Dashboard, proposed));
    }

    let mut landing = Vec::with_capacity(asked.len());
    for (kind, proposed) in asked {
        landing.push(landed(kind, proposed)?);
    }
    let landing = pointed_at_ids(landing);

    let surfaces = state.surfaces(caller);
    replaceable(&surfaces, &landing).await?;

    let batch: Vec<Definition> = landing
        .iter()
        .map(|item| Definition::new(item.kind, item.id.clone(), item.body.clone()))
        .collect();
    surfaces
        .check_batch(&batch)
        .await
        .map_err(crate::api::definitions::custom_error)?;

    let (created, updated) = reported(&landing);
    let changes: Vec<Change> = landing
        .into_iter()
        .flat_map(|item| changes_of(item, caller))
        .collect();
    state
        .definitions()
        .apply(&changes)
        .await
        .map_err(ChatApiError::definition_store_error)?;

    Ok(Json(ChatCreatedResponse {
        reply,
        created,
        updated,
    })
    .into_response())
}

/// Refuses a proposal replacing a dashboard the caller may not change, as
/// missing when they cannot see it.
async fn replaceable(surfaces: &Surfaces<'_>, landing: &[Landing]) -> Result<(), CanonicalError> {
    let replaced = landing
        .iter()
        .filter(|item| item.replaces && item.kind == DefinitionKind::Dashboard);

    for item in replaced {
        surfaces
            .changeable(&item.id)
            .await
            .map_err(crate::api::definitions::custom_error)?;
    }

    Ok(())
}

/// The id an item is stored under: the one it replaces, or a fresh one.
fn landed(kind: DefinitionKind, proposed: Proposed) -> Result<Landing, CanonicalError> {
    let name = DefinitionLabel::parse(&proposed.name).map_err(ChatApiError::definition_error)?;
    let (id, replaces) = match proposed.id.as_deref() {
        Some(raw) => {
            let id = DefinitionId::parse(raw).map_err(|_| ChatApiError::malformed_id(kind, raw))?;
            (id, true)
        }
        None => (DefinitionId::generate(), false),
    };

    Ok(Landing {
        kind,
        id,
        name,
        replaces,
        body: proposed.body,
    })
}

/// Every body, with each reference to another item of the proposal by its
/// name now naming that item's id. A reference to anything else is an id of
/// something built already, and is left as it is.
fn pointed_at_ids(landing: Vec<Landing>) -> Vec<Landing> {
    let keys: Vec<(DefinitionKind, String, String)> = landing
        .iter()
        .map(|item| {
            let key = item.name.as_str().to_owned();
            (item.kind, key, item.id.as_str().to_owned())
        })
        .collect();

    landing
        .into_iter()
        .map(|mut item| {
            let pairs: Vec<(&str, &str)> = keys
                .iter()
                .filter(|(kind, _, _)| kinds::referred_to_by(*kind).contains(&item.kind))
                .map(|(_, key, id)| (key.as_str(), id.as_str()))
                .collect();
            item.body = kinds::rename_references(item.kind, item.body, &pairs);
            item
        })
        .collect()
}

fn changes_of(item: Landing, caller: PersonId) -> Vec<Change> {
    if item.replaces {
        return vec![Change::Replace(item.kind, item.id, item.name, item.body)];
    }

    creating(item.kind, &item.id, &item.name, item.body, caller)
}

/// What was created and what was replaced, by id and name.
fn reported(landing: &[Landing]) -> (Written, Written) {
    let mut created = Written::default();
    let mut updated = Written::default();

    for item in landing {
        let reported = if item.replaces {
            &mut updated
        } else {
            &mut created
        };
        let one = WrittenOne {
            id: item.id.as_str().to_owned(),
            name: item.name.as_str().to_owned(),
        };
        match item.kind {
            DefinitionKind::Metric => reported.metric = Some(one),
            DefinitionKind::Widget => reported.widgets.push(one),
            DefinitionKind::Dashboard => reported.dashboard = Some(one),
        }
    }

    (created, updated)
}

/// What an answer says when its query found nothing.
///
/// The reply is written before the query runs, so a model that promised rows
/// and got none would present its own sentence over an empty result. It does
/// not get to be the one talking about data that is not there.
fn no_rows(table: Option<&str>) -> String {
    match table {
        Some(table) => format!("No data: that query returned no rows from {table}."),
        None => "No data: that query returned no rows.".to_owned(),
    }
}

/// The rows behind an answer.
///
/// No query means the reply stands on its own - a question about what data
/// exists is answered by the table list in the prompt.
async fn answered_with(
    state: &AppState,
    query: Option<MetricQuery>,
) -> Result<Option<RunResult>, CanonicalError> {
    let Some(query) = query else {
        return Ok(None);
    };

    let answered = state
        .metric_runs()
        .answer(&query)
        .await
        .map_err(crate::api::definitions::custom_error)?;

    Ok(Some(answered))
}

fn chat_error(error: ChatError) -> CanonicalError {
    match error {
        ChatError::Json(source) => ChatApiError::invalid_argument()
            .with_field_violation("reply", source.to_string(), "INVALID")
            .create(),
        ChatError::Metric(source) => ChatApiError::invalid_argument()
            .with_field_violation("reply", source.to_string(), "INVALID")
            .create(),
        ChatError::UnknownDataset { .. } | ChatError::NoDataset { .. } => {
            ChatApiError::invalid_argument()
                .with_field_violation("query", error.to_string(), "INVALID")
                .create()
        }
        ChatError::TooManyLookups => {
            tracing::warn!("the model exhausted its schema lookups without answering");
            CanonicalError::internal("the model did not answer").create()
        }
        ChatError::EmptyCreate | ChatError::SharedName { .. } => ChatApiError::invalid_argument()
            .with_field_violation("reply", error.to_string(), "INVALID")
            .create(),
        ChatError::TokenRejected => {
            tracing::error!("the configured anthropic token was rejected upstream");
            CanonicalError::internal("chat is not configured correctly").create()
        }
        ChatError::Unavailable => {
            tracing::warn!("the model was unavailable");
            CanonicalError::internal("the model is unavailable right now").create()
        }
        ChatError::Timeout => {
            ChatApiError::deadline_exceeded("the model did not answer in time").create()
        }
        ChatError::Failed => {
            tracing::error!("the model call failed");
            CanonicalError::internal("chat failed").create()
        }
        ChatError::NoKey => {
            tracing::error!("the assistant was asked to answer with no anthropic token set");
            CanonicalError::internal("the assistant is not configured on this instance").create()
        }
    }
}

#[cfg(test)]
mod tests;
