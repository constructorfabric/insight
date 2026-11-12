use toolkit_canonical_errors::{CanonicalError, resource_error};

/// Resource namespace for `/v1/connector-health*` (the operator's view of what
/// the data mover reports about every connector's syncs).
#[resource_error("gts.cf.insight.analytics_api.connector_health.v1~")]
pub struct ConnectorHealthError;

/// Resource namespace for `/v1/ingestion*` (the admin ingestion-intensity read
/// over the bronze ops view). Infrastructure-wide, not tenant-scoped.
#[resource_error("gts.cf.insight.analytics_api.ingestion.v1~")]
pub struct IngestionError;

pub(super) fn identity_unavailable() -> CanonicalError {
    CanonicalError::internal("failed to verify caller permissions").create()
}

#[derive(Debug)]
#[toolkit_canonical_errors::resource_error("gts.cf.insight.connectors_api.schedule.v1~")]
pub(super) struct ConnectorScheduleError;
