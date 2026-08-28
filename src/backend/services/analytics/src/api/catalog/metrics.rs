use axum::Json;
use toolkit_canonical_errors::CanonicalError;

use crate::domain::metric_query::capability::{MetricCatalogResponse, describe};
use crate::domain::metric_query::product_metric_catalog;

// SAFETY: the answer is a projection of definitions compiled into the binary,
// so this handler takes neither a security context nor a backend — there is no
// tenant value it could disclose.
pub async fn list_metric_catalog() -> Result<Json<MetricCatalogResponse>, CanonicalError> {
    let catalog = product_metric_catalog().map_err(|error| {
        tracing::error!(%error, "the shipped definitions did not load");
        CanonicalError::internal("metric definitions unavailable").create()
    })?;

    let response = describe(catalog).map_err(|error| {
        tracing::error!(%error, "a shipped metric did not resolve its inputs");
        CanonicalError::internal("metric definitions unavailable").create()
    })?;

    Ok(Json(response))
}
