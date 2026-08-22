//! `/v1/ai/credentials` — the caller's own Anthropic key.
//!
//! INVARIANT: no response, log line, or error on this path carries the key.
//! Callers learn only whether one is stored and its last four characters.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use sea_orm::sea_query::OnConflict;
use sea_orm::{EntityTrait, Set};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::super::AppState;
use super::super::error::AiError;
use super::{ensure_enabled, read_error, write_error};
use crate::domain::ai::crypto;
use crate::domain::ai::dto::{AiCredentialResponse, ApiToken, PutCredentialRequest};
use crate::infra::db::entities::ai_credentials;

/// `GET /v1/ai/credentials` — whether the caller has a key stored.
pub async fn get_credential(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    ensure_enabled(&state)?;

    let row = load(&state, ctx.subject_tenant_id(), ctx.subject_id()).await?;

    Ok(Json(match row {
        Some(row) => AiCredentialResponse {
            configured: true,
            hint: row.hint,
        },
        None => AiCredentialResponse {
            configured: false,
            hint: String::new(),
        },
    }))
}

/// `PUT /v1/ai/credentials` — store or replace the caller's key.
pub async fn put_credential(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<PutCredentialRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    ensure_enabled(&state)?;

    let token = ApiToken::parse(&req.token).map_err(|rejected| {
        AiError::invalid_argument()
            .with_field_violation("token", "the Anthropic key to store", rejected.reason())
            .create()
    })?;

    let key = state.config.ai_assist.encryption_key().map_err(|error| {
        tracing::error!(error = %error, "AI assist is enabled with an unusable encryption key");
        CanonicalError::internal("failed to store the key").create()
    })?;

    let tenant = ctx.subject_tenant_id();
    let person = ctx.subject_id();
    let sealed = crypto::seal(&key, tenant, person, token.as_str()).map_err(|error| {
        tracing::error!(error = %error, "sealing the caller's key failed");
        CanonicalError::internal("failed to store the key").create()
    })?;

    let hint = crypto::hint(token.as_str());
    let now = Utc::now();

    // One statement, not read-then-write: two saves racing (a double-clicked
    // Save) would otherwise both find no row and the second would collide on
    // the primary key.
    let row = ai_credentials::ActiveModel {
        insight_tenant_id: Set(tenant),
        person_id: Set(person),
        nonce: Set(sealed.nonce),
        ciphertext: Set(sealed.ciphertext),
        hint: Set(hint.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    };

    ai_credentials::Entity::insert(row)
        .on_conflict(
            OnConflict::columns([
                ai_credentials::Column::InsightTenantId,
                ai_credentials::Column::PersonId,
            ])
            .update_columns([
                ai_credentials::Column::Nonce,
                ai_credentials::Column::Ciphertext,
                ai_credentials::Column::Hint,
                ai_credentials::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec(&state.db)
        .await
        .map_err(|e| write_error(&e, "credential save"))?;

    Ok(Json(AiCredentialResponse {
        configured: true,
        hint,
    }))
}

/// `DELETE /v1/ai/credentials` — forget the caller's key.
pub async fn delete_credential(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    ensure_enabled(&state)?;

    ai_credentials::Entity::delete_by_id((ctx.subject_tenant_id(), ctx.subject_id()))
        .exec(&state.db)
        .await
        .map_err(|e| write_error(&e, "credential delete"))?;

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn load(
    state: &AppState,
    tenant: Uuid,
    person: Uuid,
) -> Result<Option<ai_credentials::Model>, CanonicalError> {
    ai_credentials::Entity::find_by_id((tenant, person))
        .one(&state.db)
        .await
        .map_err(|e| read_error(&e, "credential read"))
}
