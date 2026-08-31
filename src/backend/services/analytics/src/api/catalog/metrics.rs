use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use toolkit_canonical_errors::CanonicalError;

use crate::api::AppState;
use crate::domain::metric_query::capability::{MetricCatalogResponse, describe};
use crate::domain::metric_query::product_metric_catalog;

// SAFETY: the answer is a projection of definitions compiled into the binary
// plus this installation's own gate, so it takes no security context — there is
// no tenant value it could disclose.
pub async fn list_metric_catalog(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<MetricCatalogResponse>, CanonicalError> {
    let catalog = product_metric_catalog().map_err(|error| {
        tracing::error!(%error, "the shipped definitions did not load");
        CanonicalError::internal("metric definitions unavailable").create()
    })?;

    let response =
        describe(catalog, state.config.metric_catalog.tenant_metrics_enabled).map_err(|error| {
            tracing::error!(%error, "a shipped metric did not resolve its inputs");
            CanonicalError::internal("metric definitions unavailable").create()
        })?;

    Ok(Json(response))
}
