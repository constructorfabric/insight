//! The chat endpoint: answers a question from the data, or writes
//! metric/widget/dashboard definitions.

use std::sync::Arc;

use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use toolkit::api::{OpenApiRegistry, OperationBuilder};
use toolkit_canonical_errors::{CanonicalError, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::errors::ApiErrors;
use crate::chat::{Ask, ChatError, Proposal, Turn};
use crate::domain::assistant::DatasetSchemas;
use crate::domain::definition::{Change, Definition, DefinitionKind, DefinitionName};
use crate::domain::query::metric_query::{MetricQuery, RunResult};

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
    created: CreatedNames,
    /// What already existed under these names and now holds something else.
    updated: CreatedNames,
}

#[derive(Debug, Default, Serialize)]
struct CreatedNames {
    metric: Option<String>,
    widgets: Vec<String>,
    dashboard: Option<String>,
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
    Json(request): Json<ChatRequest>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, || {
        ChatApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await?;

    let briefing = state.assistant().briefing().await;
    let schemas = DatasetSchemas::new(state.datasets(), state.definitions());
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
        } => store_proposed(&state, reply, metric, widgets, dashboard).await,
    }
}

/// Stores what the assistant proposed, or none of it.
///
/// Every name is parsed and every body checked before anything is written, so
/// one refusal leaves the reader what they had rather than half a dashboard.
async fn store_proposed(
    state: &AppState,
    reply: String,
    metric: Option<(String, serde_json::Value)>,
    widgets: Vec<(String, serde_json::Value)>,
    dashboard: Option<(String, serde_json::Value)>,
) -> Result<Response, CanonicalError> {
    let mut asked = Vec::new();
    if let Some((name, body)) = metric {
        asked.push((DefinitionKind::Metric, name, body));
    }
    for (name, body) in widgets {
        asked.push((DefinitionKind::Widget, name, body));
    }
    if let Some((name, body)) = dashboard {
        asked.push((DefinitionKind::Dashboard, name, body));
    }

    let mut writes = Vec::with_capacity(asked.len());
    for (kind, name, body) in asked {
        let parsed = DefinitionName::parse(&name).map_err(ChatApiError::definition_error)?;
        writes.push(Definition::new(kind, parsed, body));
    }

    state
        .surfaces()
        .check_batch(&writes)
        .await
        .map_err(crate::api::definitions::custom_error)?;

    let (created, updated) = named_by_novelty(state, &writes).await?;

    let batch: Vec<_> = writes
        .into_iter()
        .map(|write| Change::Put(write.kind, write.name, write.body))
        .collect();
    state
        .definitions()
        .apply(&batch)
        .await
        .map_err(ChatApiError::definition_store_error)?;

    Ok(Json(ChatCreatedResponse {
        reply,
        created,
        updated,
    })
    .into_response())
}

/// Which of these names the store does not hold yet, and which it does.
///
/// Read before the write, so a name another writer takes in between is
/// reported as created rather than replaced. The write itself is one
/// transaction either way.
async fn named_by_novelty(
    state: &AppState,
    writes: &[Definition],
) -> Result<(CreatedNames, CreatedNames), CanonicalError> {
    let mut created = CreatedNames::default();
    let mut updated = CreatedNames::default();

    for write in writes {
        let held = state
            .definitions()
            .get(write.kind, &write.name)
            .await
            .map_err(ChatApiError::definition_store_error)?
            .is_some();

        let reported = if held { &mut updated } else { &mut created };
        let held_name = write.name.as_str().to_owned();
        match write.kind {
            DefinitionKind::Metric => reported.metric = Some(held_name),
            DefinitionKind::Widget => reported.widgets.push(held_name),
            DefinitionKind::Dashboard => reported.dashboard = Some(held_name),
        }
    }

    Ok((created, updated))
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
        ChatError::UnknownDataset { .. } => ChatApiError::invalid_argument()
            .with_field_violation("query", error.to_string(), "INVALID")
            .create(),
        ChatError::TooManyLookups => {
            tracing::warn!("the model exhausted its schema lookups without answering");
            CanonicalError::internal("the model did not answer").create()
        }
        ChatError::EmptyCreate => ChatApiError::invalid_argument()
            .with_field_violation("reply", ChatError::EmptyCreate.to_string(), "INVALID")
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
