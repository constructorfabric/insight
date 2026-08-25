use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;

use super::error::ConnectorHealthError;
use super::{ADMIN_ONLY, AppState, require_admin};
use crate::domain::connector_health::{self, ConnectorHealth, read};

/// Length cap on a path-supplied connector name, so a hostile path cannot make
/// the bound parameter unbounded.
const MAX_CONNECTOR: usize = 128;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ConnectorHealthResponse {
    /// When this response was assembled. Says nothing about the facts' age.
    pub as_of: DateTime<Utc>,
    /// When a controller tick last finished, or null when none ever has.
    ///
    /// This is the page's only freshness statement, and it has to come from the
    /// recorded marker: serving the reader's own clock would read as "just now"
    /// however long ago the controller last ran.
    pub swept_at: Option<DateTime<Utc>>,
    /// False when nothing has recorded a run yet — a fresh install before the
    /// first controller cadence, or a stand where nothing records. The page says
    /// so rather than implying health.
    pub history_available: bool,
    pub connectors: Vec<ConnectorRow>,
}
impl toolkit::api::api_dto::ResponseApiDto for ConnectorHealthResponse {}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ConnectorRow {
    pub connector: String,
    pub configured: bool,
    pub last_run: Option<RunView>,
    pub last_sync: Option<SyncView>,
    pub storage: Option<StorageView>,
    pub streams: Vec<StreamView>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct RunView {
    pub status: String,
    /// The step the run reached; absent when it did not fail.
    pub step: Option<String>,
    pub started_at: DateTime<Utc>,
    pub duration_ms: u64,
    /// Outcome of this run's own transform step, when it got that far.
    pub transform_status: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct SyncView {
    /// `claimed`, `out_of_band`, or `unclaimed`. Unclaimed is unknown
    /// provenance; it is never presented as a manual sync.
    pub trigger: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    /// Null until the mover's history has been swept — only it knows how long a
    /// sync took and how much it moved.
    pub duration_ms: Option<u64>,
    pub records_moved: Option<u64>,
    /// Rows measured as delivered by this sync. Null where the measurement
    /// window had passed — absence, never a zero.
    pub rows_landed: Option<u64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct StorageView {
    pub observed_at: DateTime<Utc>,
    pub streams: u16,
    pub streams_with_data: u16,
    /// Physical rows present when observed. On a deduplicating engine this
    /// sizes a connector; it does not count entities.
    pub physical_rows: u64,
    pub bytes_on_disk: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct StreamView {
    pub stream: String,
    pub physical_rows: u64,
    pub bytes_on_disk: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ConnectorRunsResponse {
    pub connector: String,
    pub runs: Vec<RunEventView>,
}
impl toolkit::api::api_dto::ResponseApiDto for ConnectorRunsResponse {}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct RunEventView {
    pub event: String,
    pub status: String,
    pub step: Option<String>,
    /// Which writer recorded this row: `pipeline` or `sweep`. Provenance of the
    /// record, never of the sync — see `trigger` for that.
    pub origin: String,
    pub trigger: Option<String>,
    pub started_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub records_moved: u64,
    pub rows_landed: Option<u64>,
}

pub(crate) async fn get_connector_health(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let health = match read::read_health(&state.ch).await {
        Ok(health) => health,
        // A page state, not a failure: an install whose migration has not landed
        // shows an empty page that says so (spec FR-13).
        Err(read::ReadError::LedgerAbsent) => Vec::new(),
        Err(error @ read::ReadError::Clickhouse(_)) => return Err(read_failed(&error)),
    };

    let swept_at = match read::read_swept_at(&state.ch).await {
        Ok(swept_at) => swept_at,
        Err(read::ReadError::LedgerAbsent) => None,
        Err(error @ read::ReadError::Clickhouse(_)) => return Err(read_failed(&error)),
    };

    let history_available = health
        .iter()
        .any(|c| c.last_run.is_some() || c.last_sync.is_some());

    Ok(Json(ConnectorHealthResponse {
        as_of: Utc::now(),
        swept_at,
        history_available,
        connectors: health.into_iter().map(connector_row).collect(),
    }))
}

pub(crate) async fn get_connector_runs(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(connector): Path<String>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let connector = super::clip(&connector, MAX_CONNECTOR);
    let rows = match read::read_history(&state.ch, &connector).await {
        Ok(rows) => rows,
        Err(read::ReadError::LedgerAbsent) => Vec::new(),
        Err(error @ read::ReadError::Clickhouse(_)) => return Err(read_failed(&error)),
    };

    Ok(Json(ConnectorRunsResponse {
        connector,
        runs: rows.into_iter().map(run_event_view).collect(),
    }))
}

fn connector_row(health: ConnectorHealth) -> ConnectorRow {
    ConnectorRow {
        connector: health.connector,
        configured: health.configured,
        last_run: health.last_run.map(|run| RunView {
            status: run.status,
            step: non_empty(run.step),
            started_at: run.started_at,
            duration_ms: run.duration_ms,
            transform_status: run.transform_status,
        }),
        last_sync: health.last_sync.map(|sync| SyncView {
            trigger: sync.claim.as_str().to_owned(),
            status: sync.status,
            started_at: sync.started_at,
            duration_ms: sync.duration_ms,
            records_moved: sync.records_moved,
            rows_landed: sync.rows_landed,
        }),
        storage: health.storage.map(|storage| StorageView {
            observed_at: storage.observed_at,
            streams: storage.streams,
            streams_with_data: storage.streams_with_data,
            physical_rows: storage.rows_total,
            bytes_on_disk: storage.bytes_on_disk,
        }),
        streams: health
            .streams
            .into_iter()
            .map(|stream| StreamView {
                stream: stream.stream,
                physical_rows: stream.rows_total,
                bytes_on_disk: stream.bytes_on_disk,
            })
            .collect(),
    }
}

fn run_event_view(row: read::HistoryRow) -> RunEventView {
    RunEventView {
        trigger: non_empty(row.claim.clone())
            .map(|claim| connector_health::Claim::parse(&claim).as_str().to_owned()),
        event: row.event,
        status: row.status,
        step: non_empty(row.step),
        origin: row.origin,
        started_at: row.started_at,
        duration_ms: row.duration_ms,
        records_moved: row.records_moved,
        rows_landed: row.has_measurement.then_some(row.rows_landed_or_zero),
    }
}

/// The ledger stores an empty string where a column does not apply; the wire
/// says absent, so no reader has to know that convention.
fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn read_failed(error: &read::ReadError) -> CanonicalError {
    tracing::error!(error = %error, "connector health read failed");
    CanonicalError::internal("connector health query failed").create()
}

fn admin_only() -> CanonicalError {
    ConnectorHealthError::permission_denied()
        .with_reason(ADMIN_ONLY)
        .create()
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "a failed unwrap is the test failing")]
mod tests {
    use chrono::TimeZone;
    use toolkit_canonical_errors::Problem;

    use super::*;
    use crate::domain::connector_health::{Claim, StorageFacts, SyncFacts};

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 15, 9, 0, 0).unwrap()
    }

    fn health(last_sync: Option<SyncFacts>) -> ConnectorHealth {
        ConnectorHealth {
            connector: "example-tool".to_owned(),
            configured: true,
            last_run: None,
            last_sync,
            storage: Some(StorageFacts {
                observed_at: at(),
                streams: 12,
                streams_with_data: 8,
                rows_total: 4_200_000,
                bytes_on_disk: 314_572_800,
            }),
            streams: vec![],
        }
    }

    fn sync(claim: Claim, rows_landed: Option<u64>) -> SyncFacts {
        SyncFacts {
            claim,
            status: "ok".to_owned(),
            started_at: at(),
            duration_ms: Some(310_000),
            records_moved: Some(12_400),
            rows_landed,
        }
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

    #[test]
    fn the_wire_reports_the_trigger_not_which_writer_recorded_the_row() {
        let row = connector_row(health(Some(sync(Claim::OutOfBand, None))));

        assert_eq!(row.last_sync.unwrap().trigger, "out_of_band");
    }

    #[test]
    fn an_unmeasured_delivery_serialises_as_null_not_as_zero() {
        let row = connector_row(health(Some(sync(Claim::OutOfBand, None))));
        let json = serde_json::to_value(row.last_sync).unwrap();

        assert!(
            json["rows_landed"].is_null(),
            "absence must not read as zero delivery"
        );
    }

    #[test]
    fn an_uncounted_sync_serialises_its_counters_as_null() {
        // Until the mover's history has been swept nobody knows what the sync
        // moved. Reporting the pipeline row's zeros would print "0 recorded /
        // 4200 landed" — a measurement nobody took.
        let pending = SyncFacts {
            records_moved: None,
            duration_ms: None,
            ..sync(Claim::Claimed, Some(4_200))
        };
        let json = serde_json::to_value(connector_row(health(Some(pending))).last_sync).unwrap();

        assert!(json["records_moved"].is_null());
        assert!(json["duration_ms"].is_null());
        assert_eq!(json["rows_landed"], 4_200);
    }

    #[test]
    fn a_measured_zero_delivery_serialises_as_zero() {
        let row = connector_row(health(Some(sync(Claim::Claimed, Some(0)))));
        let json = serde_json::to_value(row.last_sync).unwrap();

        assert_eq!(json["rows_landed"], 0);
        assert_eq!(
            json["records_moved"], 12_400,
            "the pairing is the whole point"
        );
    }

    #[test]
    fn storage_rows_are_labelled_physical_so_nothing_reads_them_as_entities() {
        let row = connector_row(health(None));
        let json = serde_json::to_value(row.storage).unwrap();

        assert_eq!(json["physical_rows"], 4_200_000);
        assert!(json.get("rows_total").is_none());
    }

    #[test]
    fn a_column_that_does_not_apply_is_absent_rather_than_an_empty_string() {
        assert_eq!(non_empty(String::new()), None);
        assert_eq!(non_empty("resolve".to_owned()), Some("resolve".to_owned()));
    }

    #[test]
    fn an_unknown_trigger_word_from_the_ledger_reads_as_unclaimed() {
        let row = read::HistoryRow {
            event: "sync.completed".to_owned(),
            status: "ok".to_owned(),
            step: String::new(),
            origin: "sweep".to_owned(),
            claim: "something-new".to_owned(),
            started_at: at(),
            duration_ms: 1,
            records_moved: 0,
            rows_landed_or_zero: 0,
            has_measurement: false,
        };

        assert_eq!(run_event_view(row).trigger.as_deref(), Some("unclaimed"));
    }
}
