use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, Path},
};
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    AppState,
    canonical_json::CanonicalJson,
    gate::require_admin,
    resolution::{AccountRef, correction_error},
};
use crate::correction_runner;
use crate::domain::seed::SourceAccountKey;

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ProfileSourceResponse {
    pub person_id: Uuid,
    pub account: AccountRef,
}
impl toolkit::api::api_dto::ResponseApiDto for ProfileSourceResponse {}
impl toolkit::api::api_dto::RequestApiDto for AccountRef {}

pub(crate) async fn select(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(person_id): Path<Uuid>,
    CanonicalJson(account): CanonicalJson<AccountRef>,
) -> Result<Json<ProfileSourceResponse>, CanonicalError> {
    let author = require_admin(&state.db, &ctx).await?;
    let tenant = ctx.subject_tenant_id();
    let _guard = correction_runner::lock(&state.config, tenant)
        .await
        .map_err(correction_error)?;
    correction_runner::select_profile(
        &state.db,
        &state.config,
        tenant,
        author,
        person_id,
        &SourceAccountKey::from(&account),
    )
    .await
    .map_err(correction_error)?;
    Ok(Json(ProfileSourceResponse { person_id, account }))
}
