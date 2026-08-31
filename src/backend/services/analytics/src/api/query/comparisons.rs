use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::http::HeaderMap;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use crate::api::AppState;
use crate::domain::metric_query::comparisons::{
    ComparisonsRequest, ComparisonsResponse, answer, validate_request,
};
use crate::domain::metric_query::product_metric_catalog;
use crate::domain::person_visibility::authorize_person_ids;

pub async fn query_comparisons(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Json(req): Json<ComparisonsRequest>,
) -> Result<Json<ComparisonsResponse>, CanonicalError> {
    let catalog = product_metric_catalog().map_err(|error| {
        tracing::error!(%error, "the shipped definitions did not load");
        CanonicalError::internal("metric definitions unavailable").create()
    })?;

    // INVARIANT: no tenant-metric gate is needed here — validation refuses a
    // tenant-grain metric outright, so nothing keyed by the tenant is compared,
    // and the population itself is disclosed only as aggregates.
    let batch = validate_request(catalog, req)?;

    // SAFETY: every target is checked individually before a single read is
    // planned. The population is never authorized because it is never
    // disclosed — only its size and its quantiles leave this service.
    authorize_person_ids(
        &state.identity,
        &ctx,
        crate::api::forwarded_authorization(&headers),
        &batch.target_ids(),
    )
    .await?;

    let response = answer(
        catalog,
        &state.ch,
        &state.db,
        ctx.subject_tenant_id(),
        batch,
    )
    .await?;
    Ok(Json(response))
}
