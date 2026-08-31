use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::http::HeaderMap;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use crate::api::AppState;
use crate::domain::metric_access::authorize_tenant_metrics;
use crate::domain::metric_query::product_metric_catalog;
use crate::domain::metric_query::values::{
    ValidatedBatch, ValuesRequest, ValuesResponse, answer, validate_request,
};
use crate::domain::person_visibility::authorize_person_ids;

pub async fn query_values(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Json(req): Json<ValuesRequest>,
) -> Result<Json<ValuesResponse>, CanonicalError> {
    let catalog = product_metric_catalog().map_err(|error| {
        tracing::error!(%error, "the shipped definitions did not load");
        CanonicalError::internal("metric definitions unavailable").create()
    })?;

    let batch = validate_request(catalog, req)?;
    authorize_subjects(&state, &ctx, &headers, &batch).await?;

    let response = answer(
        catalog,
        &state.ch,
        &state.db,
        state.config.measure_cache.read_enabled,
        ctx.subject_tenant_id(),
        batch,
    )
    .await?;
    Ok(Json(response))
}

// INVARIANT: the whole batch is authorized before a single read is planned, so
// a question about a person this caller may not see never reaches ClickHouse.
async fn authorize_subjects(
    state: &AppState,
    ctx: &SecurityContext,
    headers: &HeaderMap,
    batch: &ValidatedBatch,
) -> Result<(), CanonicalError> {
    if batch.asks_about_the_tenant() {
        authorize_tenant_metrics(state.config.metric_catalog.tenant_metrics_enabled)?;
    }

    // SAFETY: every person is checked individually; a batch is never a way
    // around the gate deciding which people a caller may read.
    authorize_person_ids(
        &state.identity,
        ctx,
        crate::api::forwarded_authorization(headers),
        &batch.subject_ids(),
    )
    .await
}
