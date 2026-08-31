//! Image-tag listing surface.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::response::IntoResponse;
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;

use super::AppState;
use super::experiments::require_caller;
use crate::domain::experiment::preview_tags;

/// Body of `GET /v1/images`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageListResponse {
    /// False when tag listing is disabled server-side; `tags` is then empty.
    pub configured: bool,
    /// The repository's `preview-…` tags, deduped and sorted.
    pub tags: Vec<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for ImageListResponse {}

/// `GET /v1/images` — the `preview-…` tags available in the registry.
pub async fn list_images(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_caller(&ctx)?;

    let Some(registry) = &state.registry else {
        return Ok(Json(ImageListResponse {
            configured: false,
            tags: Vec::new(),
        }));
    };

    let tags = registry.list_tags().await.map_err(tags_err)?;
    Ok(Json(ImageListResponse {
        configured: true,
        tags: preview_tags(tags),
    }))
}

#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn tags_err(e: anyhow::Error) -> CanonicalError {
    tracing::error!(error = %format!("{e:#}"), "registry tag listing failed");
    CanonicalError::service_unavailable()
        .with_detail("the image registry could not be listed")
        .create()
}
