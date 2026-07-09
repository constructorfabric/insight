//! `POST /v1/catalog/get_metrics` HTTP handler (Refs #524).
//!
//! Implements `cpt-metric-cat-component-catalog-reader`'s HTTP surface per
//! DESIGN §3.3 "Catalog Read":
//!
//! - **Auth**: bearer-token-only at the gateway (out of scope here — Q1 ack);
//!   the request-context fields `role_slug` / `team_id` come from the JSON
//!   body. `tenant_id` is NEVER taken from the body; it is resolved server-side
//!   by `tenant_middleware` (Refs #522), which populates
//!   `SecurityContext.insight_tenant_id` before we run.
//! - **Content-Type + body shape**: enforced by [`CanonicalJson`], which wraps
//!   Axum's `Json<T>` extractor and converts every `JsonRejection` variant
//!   to the canonical RFC 9457 envelope. `Content-Type: application/json`
//!   required (415 otherwise per the §3.3 CSRF model); `deny_unknown_fields`
//!   on [`GetMetricsRequest`] rejects a smuggled body-supplied `tenant_id` as
//!   a canonical 400 `invalid_argument`.

use std::sync::Arc;

use axum::extract::Extension;
use axum::response::{IntoResponse, Response};
use toolkit_canonical_errors::CanonicalError;

use super::AppState;
use super::canonical_json::CanonicalJson;
use crate::domain::catalog::response::GetMetricsRequest;
use toolkit_security::SecurityContext;

/// `POST /v1/catalog/get_metrics` handler.
///
/// # Errors
///
/// - `400 invalid_argument` — malformed body, unknown body fields (incl.
///   `tenant_id`), or other deserialization failures.
/// - `415 unsupported_media_type` — Content-Type is missing or not
///   `application/json`.
/// - `500 internal` — resolver / DB failure (Redis blips are absorbed by the
///   reader's degrade-gracefully behavior).
pub async fn get_metrics(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<GetMetricsRequest>,
) -> Response {
    let response = match state
        .catalog_reader
        .read(
            ctx.subject_tenant_id(),
            req.role_slug.as_deref(),
            req.team_id.as_deref(),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "catalog: resolver failed");
            return CanonicalError::internal("failed to resolve catalog")
                .create()
                .into_response();
        }
    };

    axum::Json(response).into_response()
}
