use toolkit_canonical_errors::CanonicalError;

use crate::api::error::MetricError;

pub(crate) fn authorize_tenant_metrics(enabled: bool) -> Result<(), CanonicalError> {
    if enabled {
        return Ok(());
    }

    Err(MetricError::permission_denied()
        .with_reason("tenant_metrics_disabled")
        .create())
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use super::*;

    #[test]
    fn tenant_metrics_are_denied_when_installation_gate_is_off() {
        let Err(error) = authorize_tenant_metrics(false) else {
            panic!("disabled tenant metrics must be denied");
        };

        assert_eq!(error.into_response().status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn tenant_metrics_are_allowed_when_installation_gate_is_on() {
        assert!(authorize_tenant_metrics(true).is_ok());
    }
}
