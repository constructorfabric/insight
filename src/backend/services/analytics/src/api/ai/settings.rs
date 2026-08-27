//! `/v1/ai/settings` — the tenant's system prompt.
//!
//! Readable by anyone signed in: it is part of every explanation they get.
//! Writable by admins. Unset means the shipped default, and DELETE is Reset.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::Utc;
use sea_orm::sea_query::OnConflict;
use sea_orm::{EntityTrait, Set};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::super::error::AiError;
use super::super::{AppState, require_admin};
use super::{admin_only_prompt, ensure_enabled, read_error, write_error};
use crate::domain::ai::dto::{AiSettingsResponse, Prose, PutSettingsRequest};
use crate::domain::ai::prompt::DEFAULT_SYSTEM_PROMPT;
use crate::infra::db::entities::ai_settings;

/// `GET /v1/ai/settings` — the prompt in force for this tenant.
pub async fn get_settings(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    ensure_enabled(&state)?;

    let stored = load(&state, ctx.subject_tenant_id()).await?;

    Ok(Json(response(stored)))
}

/// `PUT /v1/ai/settings` — replace the shipped default for this tenant.
pub async fn put_settings(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Json(req): Json<PutSettingsRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    ensure_enabled(&state)?;
    require_admin(&state, &headers, admin_only_prompt).await?;

    let prompt = Prose::parse(&req.system_prompt).map_err(|rejected| {
        AiError::invalid_argument()
            .with_field_violation(
                "system_prompt",
                "the tenant system prompt",
                rejected.reason(),
            )
            .create()
    })?;

    let tenant = ctx.subject_tenant_id();
    let now = Utc::now();
    let stored = prompt.into_inner();

    // One statement, not read-then-write — two admins saving at once would
    // otherwise race on the insert.
    let row = ai_settings::ActiveModel {
        insight_tenant_id: Set(tenant),
        system_prompt: Set(Some(stored.clone())),
        updated_at: Set(now),
    };

    ai_settings::Entity::insert(row)
        .on_conflict(
            OnConflict::column(ai_settings::Column::InsightTenantId)
                .update_columns([
                    ai_settings::Column::SystemPrompt,
                    ai_settings::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&state.db)
        .await
        .map_err(|e| write_error(&e, "system prompt save"))?;

    Ok(Json(AiSettingsResponse {
        system_prompt: stored,
        is_default: false,
    }))
}

/// `DELETE /v1/ai/settings` — back to the shipped default.
pub async fn reset_settings(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, CanonicalError> {
    ensure_enabled(&state)?;
    require_admin(&state, &headers, admin_only_prompt).await?;

    ai_settings::Entity::delete_by_id(ctx.subject_tenant_id())
        .exec(&state.db)
        .await
        .map_err(|e| write_error(&e, "system prompt reset"))?;

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn response(stored: Option<String>) -> AiSettingsResponse {
    match stored {
        Some(prompt) => AiSettingsResponse {
            system_prompt: prompt,
            is_default: false,
        },
        None => AiSettingsResponse {
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_owned(),
            is_default: true,
        },
    }
}

pub(crate) async fn load(state: &AppState, tenant: Uuid) -> Result<Option<String>, CanonicalError> {
    let row = ai_settings::Entity::find_by_id(tenant)
        .one(&state.db)
        .await
        .map_err(|e| read_error(&e, "system prompt read"))?;

    Ok(row.and_then(|row| row.system_prompt))
}
