//! `GET /v1/connector-health` HTTP handler.
//!
//! Reports what each connector has actually delivered into bronze: how many
//! streams it owns, how many carry data, their size, and when data last
//! arrived. No verdict is served — the warn/error windows every connector
//! declares are not readable from here yet, and a single instance-wide age
//! limit would misreport connectors whose own limits differ.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;

use super::AppState;
use crate::domain::connector_health;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConnectorHealthResponse {
    /// Server time the state was read at, so a client does not date the
    /// reading by its own clock.
    pub as_of: DateTime<Utc>,
    pub connectors: Vec<ConnectorRow>,
}
impl toolkit::api::api_dto::ResponseApiDto for ConnectorHealthResponse {}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConnectorRow {
    pub connector: String,
    pub namespace: String,
    pub streams: usize,
    pub streams_with_data: usize,
    /// Physical rows across active parts. On a deduplicating engine this
    /// counts rows not yet merged away, so it sizes a stream and does not
    /// count entities.
    pub rows: u64,
    /// When data last arrived. Absent when the connector has never delivered.
    pub last_write: Option<DateTime<Utc>>,
}

/// `GET /v1/connector-health` handler.
///
/// # Errors
///
/// - `500 internal` — the catalogue or extract read failed.
pub async fn get_connector_health(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<impl IntoResponse, CanonicalError> {
    let streams = connector_health::read_stream_states(&state.ch)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "connector health read failed");
            CanonicalError::internal("connector health query failed").create()
        })?;

    let connectors = connector_health::summarize(&streams)
        .into_iter()
        .map(|c| ConnectorRow {
            connector: connector_health::connector_name(&c.namespace).to_owned(),
            namespace: c.namespace,
            streams: c.streams,
            streams_with_data: c.populated_streams,
            rows: c.rows,
            last_write: c.last_write,
        })
        .collect();

    Ok(Json(ConnectorHealthResponse {
        as_of: Utc::now(),
        connectors,
    }))
}
