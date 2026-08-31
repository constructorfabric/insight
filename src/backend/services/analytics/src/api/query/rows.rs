use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::http::HeaderMap;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use crate::api::AppState;
use crate::domain::metric_access::authorize_tenant_metrics;
use crate::domain::metric_query::product_metric_catalog;
use crate::domain::metric_query::rows::{RowsRequest, RowsResponse, answer, validate_request};
use crate::domain::person_visibility::authorize_person_ids;

pub async fn query_rows(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Json(req): Json<RowsRequest>,
) -> Result<Json<RowsResponse>, CanonicalError> {
    let catalog = product_metric_catalog().map_err(|error| {
        tracing::error!(%error, "the shipped definitions did not load");
        CanonicalError::internal("metric definitions unavailable").create()
    })?;

    let request = validate_request(catalog, req)?;
    if request.asks_about_the_tenant() {
        authorize_tenant_metrics(state.config.metric_catalog.tenant_metrics_enabled)?;
    }

    // SAFETY: every subject is checked individually; a page is never a way
    // around the gate deciding which people a caller may read.
    authorize_person_ids(
        &state.identity,
        &ctx,
        crate::api::forwarded_authorization(&headers),
        request.subjects.person_ids(),
    )
    .await?;

    let response = answer(
        catalog,
        &state.ch,
        &state.db,
        ctx.subject_tenant_id(),
        request,
    )
    .await?;
    Ok(Json(response))
}
