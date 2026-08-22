use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;

use super::{ADMIN_ONLY, AppState, require_admin};
use crate::api::error::ConnectorHealthError;
use crate::domain::connector_health;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ConnectorHealthResponse {
    pub as_of: DateTime<Utc>,
    pub connectors: Vec<ConnectorRow>,
}
impl toolkit::api::api_dto::ResponseApiDto for ConnectorHealthResponse {}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ConnectorRow {
    pub connector: String,
    pub namespace: String,
    pub streams: usize,
    pub streams_with_data: usize,
    // INVARIANT: physical rows across active parts — on a deduplicating engine
    // this sizes a stream and does not count entities.
    pub rows: u64,
    pub last_write: Option<DateTime<Utc>>,
}

pub(crate) async fn get_connector_health(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let streams = connector_health::read_stream_states(&state.ch)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "connector health read failed");
            CanonicalError::internal("connector health query failed").create()
        })?;

    Ok(Json(ConnectorHealthResponse {
        as_of: Utc::now(),
        connectors: connector_rows(connector_health::summarize(&streams)),
    }))
}

fn connector_rows(states: Vec<connector_health::ConnectorState>) -> Vec<ConnectorRow> {
    states
        .into_iter()
        .map(|c| ConnectorRow {
            connector: connector_health::connector_name(&c.namespace).to_owned(),
            namespace: c.namespace,
            streams: c.streams,
            streams_with_data: c.populated_streams,
            rows: c.rows,
            last_write: c.last_write,
        })
        .collect()
}

fn admin_only() -> CanonicalError {
    ConnectorHealthError::permission_denied()
        .with_reason(ADMIN_ONLY)
        .create()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a failed unwrap is the test failing")]
mod tests {
    use chrono::TimeZone;
    use toolkit_canonical_errors::Problem;

    use super::*;
    use crate::domain::connector_health::ConnectorState;

    fn state(namespace: &str, last_write: Option<DateTime<Utc>>) -> ConnectorState {
        ConnectorState {
            namespace: namespace.to_owned(),
            streams: 4,
            populated_streams: 3,
            rows: 12,
            last_write,
        }
    }

    #[test]
    fn a_summary_row_is_reported_under_the_connector_name_not_the_schema() {
        let at = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();

        let rows = connector_rows(vec![state("bronze_example", Some(at))]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].connector, "example");
        assert_eq!(rows[0].namespace, "bronze_example");
        assert_eq!(rows[0].streams, 4);
        assert_eq!(rows[0].streams_with_data, 3);
        assert_eq!(rows[0].rows, 12);
        assert_eq!(rows[0].last_write, Some(at));
    }

    #[test]
    fn a_connector_that_never_delivered_is_reported_without_a_last_write() {
        let rows = connector_rows(vec![state("bronze_example", None)]);

        assert_eq!(rows[0].last_write, None);
    }

    #[test]
    fn a_caller_without_the_admin_role_is_refused_rather_than_served() {
        let problem = serde_json::to_value(Problem::from(admin_only())).unwrap();

        assert_eq!(problem["status"], 403);
        assert_eq!(
            problem["context"]["resource_type"],
            "gts.cf.insight.analytics_api.connector_health.v1~"
        );
    }
}
