use axum::Json;
use toolkit_canonical_errors::CanonicalError;

use crate::domain::definitions::dry_run::{
    ValidateDefinitionsRequest, ValidateDefinitionsResponse, dry_run,
};

// SAFETY: the outcome is the payload, so a set that breaks every rule is a 200
// carrying the breakages; and the handler holds no store, so nothing submitted
// can be kept.
pub async fn validate_definitions(
    Json(req): Json<ValidateDefinitionsRequest>,
) -> Result<Json<ValidateDefinitionsResponse>, CanonicalError> {
    let response = dry_run(req).map_err(|error| {
        tracing::error!(%error, "the shipped definitions did not load");
        CanonicalError::internal("metric definitions unavailable").create()
    })?;

    Ok(Json(response))
}
