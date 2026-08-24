//! `POST /v1/ai/explain` — one metric, read back in plain language.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;

use super::super::error::AiError;
use super::super::{AppState, require_admin};
use super::{admin_only_explain, context, credentials, ensure_enabled, settings};
use crate::domain::ai::crypto;
use crate::domain::ai::prompt::{self, MetricSnapshot};
use crate::infra::anthropic::AnthropicError;

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExplainRequest {
    #[serde(flatten)]
    pub snapshot: MetricSnapshot,
}
impl toolkit::api::api_dto::RequestApiDto for ExplainRequest {}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExplainResponse {
    /// The answer, as plain prose.
    pub text: String,
    /// The model that produced it.
    pub model: String,
    /// How many organisation entries fed the prompt.
    pub tenant_context_entries: usize,
    /// How many of the caller's own entries fed the prompt.
    pub person_context_entries: usize,
}
impl toolkit::api::api_dto::ResponseApiDto for ExplainResponse {}

pub async fn explain_metric(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ExplainRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    ensure_enabled(&state)?;
    if state.config.ai_assist.admin_only {
        require_admin(&state, &headers, admin_only_explain).await?;
    }

    let tenant = ctx.subject_tenant_id();
    let person = ctx.subject_id();

    let token = caller_token(&state, tenant, person).await?;
    let (tenant_entries, person_entries) = context::prompt_entries(&state, tenant, person).await?;
    let base = settings::load(&state, tenant)
        .await?
        .unwrap_or_else(|| prompt::DEFAULT_SYSTEM_PROMPT.to_owned());

    let system = prompt::build_system_prompt(&base, &tenant_entries, &person_entries);
    let user = prompt::snapshot_message(&req.snapshot);

    // INVARIANT: the permit is held across the upstream await on purpose — the
    // cap is on calls in flight, not on calls started.
    //
    // Refused rather than queued: waiting for a permit would hold the request
    // task and the decrypted key in memory behind a slow upstream, and the
    // caller would see a hang where the contract promises a "busy" answer.
    let Ok(_permit) = state.ai_calls.try_acquire() else {
        tracing::warn!("explain refused: every in-flight slot is taken");
        return Err(AiError::resource_exhausted(
            "too many explanations in flight; try again in a moment",
        )
        .with_quota_violation("ai_calls", "this instance caps concurrent model calls")
        .create());
    };

    let answer = state
        .anthropic
        .message(
            &token,
            &state.config.ai_assist.model,
            state.config.ai_assist.max_output_tokens,
            &system,
            &user,
        )
        .await
        .map_err(|error| upstream_error(&error, state.config.ai_assist.has_stand_key()))?;

    tracing::info!(
        metric_key = %req.snapshot.metric_key,
        input_tokens = answer.usage.input_tokens,
        output_tokens = answer.usage.output_tokens,
        "explained a metric"
    );

    Ok(Json(ExplainResponse {
        text: answer.text,
        model: state.config.ai_assist.model.clone(),
        tenant_context_entries: tenant_entries.len(),
        person_context_entries: person_entries.len(),
    }))
}

/// The key this call is paid for with.
///
/// The stand's own key wins where one is set: the deployment is buying the
/// answers, and a personal key left over from before should not quietly start
/// paying again.
async fn caller_token(
    state: &AppState,
    tenant: uuid::Uuid,
    person: uuid::Uuid,
) -> Result<zeroize::Zeroizing<String>, CanonicalError> {
    if state.config.ai_assist.has_stand_key() {
        return Ok(zeroize::Zeroizing::new(
            state.config.ai_assist.api_key.trim().to_owned(),
        ));
    }

    let row = credentials::load(state, tenant, person)
        .await?
        .ok_or_else(|| {
            AiError::invalid_argument()
                .with_field_violation(
                    "token",
                    "the caller's stored Anthropic key",
                    "no Anthropic key is stored for this caller",
                )
                .create()
        })?;

    let key = state.config.ai_assist.encryption_key().map_err(|error| {
        tracing::error!(error = %error, "AI assist is enabled with an unusable encryption key");
        CanonicalError::internal("failed to read the stored key").create()
    })?;

    let sealed = crypto::Sealed {
        nonce: row.nonce,
        ciphertext: row.ciphertext,
    };

    crypto::open(&key, tenant, person, &sealed).map_err(|error| {
        tracing::error!(error = %error, "the stored key could not be opened");
        CanonicalError::internal("failed to read the stored key").create()
    })
}

fn upstream_error(error: &AnthropicError, stand_key: bool) -> CanonicalError {
    match error {
        // Whose key was rejected decides who can do anything about it: telling
        // a reader to replace a key in settings they have no field for sends
        // them looking for a control that is not there.
        AnthropicError::TokenRejected if stand_key => AiError::invalid_argument()
            .with_field_violation(
                "token",
                "the Anthropic key configured for this deployment",
                "the deployment's Anthropic key was rejected; an operator has to replace it",
            )
            .create(),
        AnthropicError::TokenRejected => AiError::invalid_argument()
            .with_field_violation(
                "token",
                "the caller's stored Anthropic key",
                "the stored Anthropic key was rejected; replace it in settings",
            )
            .create(),
        AnthropicError::Unavailable | AnthropicError::Timeout => {
            AiError::resource_exhausted("the model is busy right now; try again in a moment")
                .with_quota_violation("anthropic", "the upstream refused or timed out this call")
                .create()
        }
        AnthropicError::Failed => {
            CanonicalError::internal("failed to explain this metric").create()
        }
    }
}
