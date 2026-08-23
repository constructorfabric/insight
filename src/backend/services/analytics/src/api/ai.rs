//! `/v1/ai*` — the written context, the tenant system prompt, per-person keys,
//! and the explain call.
//!
//! Every route but `/v1/ai/config` is gated on the stand switch: a deployment
//! with the feature off answers "not found" rather than advertising a surface
//! its operator did not turn on.

pub mod context;
pub mod credentials;
pub mod explain;
pub mod settings;

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::response::IntoResponse;
use toolkit_canonical_errors::CanonicalError;

use super::AppState;
use super::error::AiError;
use crate::domain::ai::dto::AiConfigResponse;

pub(crate) const ADMIN_ONLY_CONTEXT: &str = "admin role required to write organisation context";
pub(crate) const ADMIN_ONLY_EXPLAIN: &str = "admin role required to ask for an explanation";
pub(crate) const ADMIN_ONLY_PROMPT: &str = "admin role required to write the system prompt";

/// `GET /v1/ai/config` — whether this deployment offers explanations.
///
/// Deliberately answers on a stand with the feature off: "off" IS the answer
/// the SPA needs, and a 404 here would be indistinguishable from a version that
/// predates the feature.
pub async fn get_ai_config(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<impl IntoResponse, CanonicalError> {
    Ok(Json(AiConfigResponse {
        enabled: state.config.ai_assist.enabled,
        model: state.config.ai_assist.model.clone(),
        stand_key: state.config.ai_assist.has_stand_key(),
        admin_only: state.config.ai_assist.admin_only,
    }))
}

/// Refuse every other AI route when the stand switch is off.
pub(crate) fn ensure_enabled(state: &AppState) -> Result<(), CanonicalError> {
    if state.config.ai_assist.enabled {
        return Ok(());
    }
    Err(
        AiError::not_found("AI assistance is not enabled on this instance")
            .with_resource("ai_assist")
            .create(),
    )
}

pub(crate) fn admin_only_context() -> CanonicalError {
    AiError::permission_denied()
        .with_reason(ADMIN_ONLY_CONTEXT)
        .create()
}

pub(crate) fn admin_only_explain() -> CanonicalError {
    AiError::permission_denied()
        .with_reason(ADMIN_ONLY_EXPLAIN)
        .create()
}

pub(crate) fn admin_only_prompt() -> CanonicalError {
    AiError::permission_denied()
        .with_reason(ADMIN_ONLY_PROMPT)
        .create()
}

pub(crate) fn write_error(error: &sea_orm::DbErr, what: &str) -> CanonicalError {
    tracing::error!(error = %error, "AI assist write failed: {what}");
    CanonicalError::internal("failed to store AI assist data").create()
}

pub(crate) fn read_error(error: &sea_orm::DbErr, what: &str) -> CanonicalError {
    tracing::error!(error = %error, "AI assist read failed: {what}");
    CanonicalError::internal("failed to read AI assist data").create()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;
