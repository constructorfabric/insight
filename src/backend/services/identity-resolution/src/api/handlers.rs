//! Route handlers (thin controllers — DTOs + assembly live in `crate::domain`).
//!
//! `/health` + `/healthz` + `/docs` are provided by the api-gateway host gear,
//! so this service defines no health handler.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::response::IntoResponse;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::AppState;
use super::canonical_json::CanonicalJson;
use super::error::ProfileError;
use crate::domain::profile::{ResolveProfileCommand, assemble_profile};
use crate::infra::db::persons_repo;

/// `POST /v1/profiles` — resolve one identity (email or source-native id) to a
/// person, then assemble the profile.
///
/// 0 matches → 404; >1 → 409 (the .NET service used 422 `ambiguous_profile`;
/// gears canonical errors have no 422, so we map to `aborted`/409 — flagged for
/// review).
pub async fn resolve_profile(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(cmd): CanonicalJson<ResolveProfileCommand>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let value_type = cmd.value_type.trim();

    // Request validation (mirrors the .NET `ResolveProfileCommandValidator`).
    if cmd.value.trim().is_empty() {
        return Err(ProfileError::invalid_argument()
            .with_field_violation("value", "value must not be empty", "INVALID")
            .create());
    }
    if cmd.value.chars().count() > 320 {
        return Err(ProfileError::invalid_argument()
            .with_field_violation("value", "value must be at most 320 characters", "INVALID")
            .create());
    }
    if value_type == "email" && (cmd.insight_source_type.is_some() || cmd.insight_source_id.is_some())
    {
        return Err(ProfileError::invalid_argument()
            .with_field_violation(
                "insight_source_type",
                "insight_source_type / insight_source_id must be null for value_type='email'",
                "INVALID",
            )
            .create());
    }

    let person_ids = match value_type {
        "email" => persons_repo::resolve_person_ids_by_email(&state.db, tenant, &cmd.value)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "resolve by email failed");
                CanonicalError::internal("profile resolution failed").create()
            })?,
        "id" => {
            let source_type = cmd.insight_source_type.as_deref().ok_or_else(|| {
                ProfileError::invalid_argument()
                    .with_field_violation(
                        "insight_source_type",
                        "insight_source_type is required for value_type='id'",
                        "REQUIRED",
                    )
                    .create()
            })?;
            let source_id = cmd.insight_source_id.ok_or_else(|| {
                ProfileError::invalid_argument()
                    .with_field_violation(
                        "insight_source_id",
                        "insight_source_id is required for value_type='id'",
                        "REQUIRED",
                    )
                    .create()
            })?;
            persons_repo::resolve_person_ids_by_source_id(
                &state.db,
                tenant,
                source_type,
                source_id,
                &cmd.value,
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "resolve by source id failed");
                CanonicalError::internal("profile resolution failed").create()
            })?
        }
        _ => {
            return Err(ProfileError::invalid_argument()
                .with_field_violation(
                    "value_type",
                    "value_type must be 'email' or 'id'",
                    "INVALID",
                )
                .create());
        }
    };

    match person_ids.as_slice() {
        [] => Err(ProfileError::not_found("person not found")
            .with_resource(cmd.value)
            .create()),
        [person_id] => {
            let observations =
                persons_repo::fetch_person_observations(&state.db, tenant, *person_id)
                    .await
                    .map_err(|e| {
                        tracing::error!(error = %e, "fetch person observations failed");
                        CanonicalError::internal("profile assembly failed").create()
                    })?;
            Ok(Json(assemble_profile(*person_id, tenant, observations)))
        }
        _ => Err(ProfileError::aborted("email resolves to multiple persons")
            .with_reason("AMBIGUOUS_PROFILE")
            .create()),
    }
}
