//! The ledger reads behind the connector-health page.
//!
//! Three statements over one table, and nothing else: no mover, no cluster API,
//! and no access to raw connector data in any form. What upstream cannot reach
//! only makes these facts older, never absent (spec FR-14).

use chrono::{DateTime, Utc};
use clickhouse::Row;
use serde::Deserialize;

use super::{Claim, ConnectorHealth, RunFacts, StorageFacts, StreamFacts, SyncFacts, summarize};

const LEDGER: &str = "ingestion_runs.pipeline_events";

/// Recent runs a drill-down shows for one connector.
const HISTORY_LIMIT: u64 = 50;

/// Claim precedence, then recency — the same resolution the sweep uses, so the
/// two sides of the seam never disagree about which row wins.
const CLAIM_RANK: &str = "multiIf(claim = 'claimed', 3, claim = 'out_of_band', 2, 1)";

/// One row per connector: its newest terminal run, and the transform outcome
/// belonging to that same run rather than to whichever ran last.
const RUNS_SQL: &str = "\
    SELECT connector,
           argMax(status, ts)      AS status,
           argMax(step, ts)        AS step,
           argMax(started_at, ts)  AS started_at,
           argMax(duration_ms, ts) AS duration_ms,
           argMax(run_id, ts)      AS run_id
    FROM {LEDGER}
    WHERE event = 'run.finished' AND connector != ''
    GROUP BY connector";

const TRANSFORMS_SQL: &str = "\
    SELECT connector, argMax(status, ts) AS status, argMax(run_id, ts) AS run_id
    FROM {LEDGER}
    WHERE event = 'transform.completed' AND connector != ''
    GROUP BY connector";

/// One row per connector: the newest sync, resolved by claim precedence.
///
/// Counters come from whichever row carries them and the measurement from
/// whichever took it — the sweep records the first, the pipeline the second, and
/// neither row is complete alone.
const SYNCS_SQL: &str = "\
    SELECT connector,
           argMax(claim, (rank, ts))         AS claim,
           argMax(status, (rank, ts))        AS status,
           any(started_at)                   AS started_at,
           max(duration_ms)                  AS duration_ms,
           max(records_moved)                AS records_moved,
           max(rows_landed)                  AS rows_landed,
           max(has_measurement)              AS has_measurement
    FROM (
      SELECT connector, job_id, claim, status, started_at, duration_ms,
             records_moved, coalesce(rows_landed, 0) AS rows_landed,
             rows_landed IS NOT NULL AS has_measurement, ts,
             {CLAIM_RANK} AS rank
      FROM {LEDGER}
      WHERE event = 'sync.completed' AND connector != ''
        AND (connector, job_id) IN (
          SELECT connector, argMax(job_id, started_at)
          FROM {LEDGER}
          WHERE event = 'sync.completed' AND connector != ''
          GROUP BY connector
        )
    )
    GROUP BY connector";

/// The newest storage observation per connector, and its per-stream rows.
const STORAGE_SQL: &str = "\
    SELECT connector, ts AS observed_at, streams, streams_with_data, rows_total, bytes_on_disk
    FROM {LEDGER}
    WHERE event = 'storage.observed' AND stream = '' AND connector != ''
      AND (connector, run_id) IN (
        SELECT connector, argMax(run_id, ts)
        FROM {LEDGER}
        WHERE event = 'storage.observed' AND stream = '' AND connector != ''
        GROUP BY connector
      )";

const STREAMS_SQL: &str = "\
    SELECT connector, stream, rows_total, bytes_on_disk
    FROM {LEDGER}
    WHERE event = 'storage.observed' AND stream != '' AND connector != ''
      AND (connector, run_id) IN (
        SELECT connector, argMax(run_id, ts)
        FROM {LEDGER}
        WHERE event = 'storage.observed' AND stream != '' AND connector != ''
        GROUP BY connector
      )";

/// The configured set is the membership of the newest SEALED snapshot.
///
/// Sealed matters: without keying on the marker, a snapshot still being written
/// would read as the whole set, and a connector removed a moment ago would come
/// back for one tick.
const CONFIGURED_SQL: &str = "\
    SELECT connector
    FROM {LEDGER}
    WHERE event = 'connector.configured'
      AND run_id = (
        SELECT argMax(run_id, ts) FROM {LEDGER} WHERE event = 'sweep.completed'
      )";

const HISTORY_SQL: &str = "\
    SELECT event, status, step, origin, claim, started_at, duration_ms,
           records_moved, coalesce(rows_landed, 0) AS rows_landed,
           rows_landed IS NOT NULL AS has_measurement
    FROM {LEDGER}
    WHERE connector = ? AND event != 'storage.observed'
    ORDER BY ts DESC
    LIMIT {HISTORY_LIMIT}";

fn sql(template: &str) -> String {
    template
        .replace("{LEDGER}", LEDGER)
        .replace("{CLAIM_RANK}", CLAIM_RANK)
        .replace("{HISTORY_LIMIT}", &HISTORY_LIMIT.to_string())
}

#[derive(Debug, Row, Deserialize)]
struct RunRow {
    connector: String,
    status: String,
    step: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    started_at: DateTime<Utc>,
    duration_ms: u64,
    run_id: String,
}

#[derive(Debug, Row, Deserialize)]
struct TransformRow {
    connector: String,
    status: String,
    run_id: String,
}

#[derive(Debug, Row, Deserialize)]
struct SyncRow {
    connector: String,
    claim: String,
    status: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    started_at: DateTime<Utc>,
    duration_ms: u64,
    records_moved: u64,
    rows_landed: u64,
    has_measurement: bool,
}

#[derive(Debug, Row, Deserialize)]
struct StorageRow {
    connector: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    observed_at: DateTime<Utc>,
    streams: u16,
    streams_with_data: u16,
    rows_total: u64,
    bytes_on_disk: u64,
}

#[derive(Debug, Row, Deserialize)]
struct StreamRow {
    connector: String,
    stream: String,
    rows_total: u64,
    bytes_on_disk: u64,
}

#[derive(Debug, Row, Deserialize)]
struct ConfiguredRow {
    connector: String,
}

#[derive(Debug, Row, Deserialize)]
pub(crate) struct HistoryRow {
    pub(crate) event: String,
    pub(crate) status: String,
    pub(crate) step: String,
    pub(crate) origin: String,
    pub(crate) claim: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    pub(crate) started_at: DateTime<Utc>,
    pub(crate) duration_ms: u64,
    pub(crate) records_moved: u64,
    pub(crate) rows_landed: u64,
    pub(crate) has_measurement: bool,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReadError {
    /// The ledger is absent — the migration has not run here yet. A page state,
    /// not a failure (spec FR-13).
    #[error("run ledger is not present")]
    LedgerAbsent,
    #[error(transparent)]
    Clickhouse(#[from] clickhouse::error::Error),
}

fn absent_ledger(error: &clickhouse::error::Error) -> bool {
    // The one error that is a deployment state rather than a fault: an install
    // whose migration has not landed answers this way until it does.
    error.to_string().contains("UNKNOWN_TABLE") || error.to_string().contains("UNKNOWN_DATABASE")
}

async fn fetch<T>(ch: &insight_clickhouse::Client, statement: &str) -> Result<Vec<T>, ReadError>
where
    T: clickhouse::RowOwned + clickhouse::RowRead,
{
    ch.query(statement).fetch_all::<T>().await.map_err(|error| {
        if absent_ledger(&error) {
            return ReadError::LedgerAbsent;
        }
        ReadError::Clickhouse(error)
    })
}

/// Every connector's recorded health, ordered by what needs attention.
pub(crate) async fn read_health(
    ch: &insight_clickhouse::Client,
) -> Result<Vec<ConnectorHealth>, ReadError> {
    let runs = fetch::<RunRow>(ch, &sql(RUNS_SQL)).await?;
    let transforms = fetch::<TransformRow>(ch, &sql(TRANSFORMS_SQL)).await?;
    let syncs = fetch::<SyncRow>(ch, &sql(SYNCS_SQL)).await?;
    let storage = fetch::<StorageRow>(ch, &sql(STORAGE_SQL)).await?;
    let streams = fetch::<StreamRow>(ch, &sql(STREAMS_SQL)).await?;
    let configured = fetch::<ConfiguredRow>(ch, &sql(CONFIGURED_SQL)).await?;

    Ok(summarize(
        run_facts(runs, &transforms),
        sync_facts(syncs),
        storage_facts(storage),
        stream_facts(streams),
        &configured
            .into_iter()
            .map(|row| row.connector)
            .collect::<Vec<_>>(),
    ))
}

/// Recent events for one connector, newest first.
pub(crate) async fn read_history(
    ch: &insight_clickhouse::Client,
    connector: &str,
) -> Result<Vec<HistoryRow>, ReadError> {
    ch.query(&sql(HISTORY_SQL))
        .bind(connector)
        .fetch_all::<HistoryRow>()
        .await
        .map_err(|error| {
            if absent_ledger(&error) {
                return ReadError::LedgerAbsent;
            }
            ReadError::Clickhouse(error)
        })
}

/// INVARIANT: a transform outcome belongs to a run only when it shares its id.
/// Pairing the newest of each instead would report an old transform against a
/// fresh run — a stalled downstream layer that has since recovered, or worse.
fn run_facts(runs: Vec<RunRow>, transforms: &[TransformRow]) -> Vec<(String, RunFacts)> {
    runs.into_iter()
        .map(|row| {
            let transform_status = transforms
                .iter()
                .find(|t| t.connector == row.connector && t.run_id == row.run_id)
                .map(|t| t.status.clone());
            (
                row.connector,
                RunFacts {
                    status: row.status,
                    step: row.step,
                    started_at: row.started_at,
                    duration_ms: row.duration_ms,
                    transform_status,
                },
            )
        })
        .collect()
}

fn sync_facts(syncs: Vec<SyncRow>) -> Vec<(String, SyncFacts)> {
    syncs
        .into_iter()
        .map(|row| {
            (
                row.connector,
                SyncFacts {
                    claim: Claim::parse(&row.claim),
                    status: row.status,
                    started_at: row.started_at,
                    duration_ms: row.duration_ms,
                    records_moved: row.records_moved,
                    rows_landed: row.has_measurement.then_some(row.rows_landed),
                },
            )
        })
        .collect()
}

fn storage_facts(storage: Vec<StorageRow>) -> Vec<(String, StorageFacts)> {
    storage
        .into_iter()
        .map(|row| {
            (
                row.connector,
                StorageFacts {
                    observed_at: row.observed_at,
                    streams: row.streams,
                    streams_with_data: row.streams_with_data,
                    rows_total: row.rows_total,
                    bytes_on_disk: row.bytes_on_disk,
                },
            )
        })
        .collect()
}

fn stream_facts(streams: Vec<StreamRow>) -> Vec<(String, StreamFacts)> {
    streams
        .into_iter()
        .map(|row| {
            (
                row.connector,
                StreamFacts {
                    stream: row.stream,
                    rows_total: row.rows_total,
                    bytes_on_disk: row.bytes_on_disk,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn run_row(connector: &str, run_id: &str) -> RunRow {
        RunRow {
            connector: connector.to_owned(),
            status: "ok".to_owned(),
            step: "done".to_owned(),
            started_at: Utc.timestamp_opt(0, 0).unwrap(),
            duration_ms: 1,
            run_id: run_id.to_owned(),
        }
    }

    fn transform_row(connector: &str, run_id: &str, status: &str) -> TransformRow {
        TransformRow {
            connector: connector.to_owned(),
            run_id: run_id.to_owned(),
            status: status.to_owned(),
        }
    }

    #[test]
    fn a_transform_outcome_attaches_only_to_its_own_run() {
        let facts = run_facts(
            vec![run_row("alpha", "wf-2")],
            &[transform_row("alpha", "wf-1", "failed")],
        );

        assert_eq!(
            facts[0].1.transform_status, None,
            "a previous run's transform is not this run's"
        );
    }

    #[test]
    fn a_transform_outcome_from_the_same_run_attaches() {
        let facts = run_facts(
            vec![run_row("alpha", "wf-2")],
            &[transform_row("alpha", "wf-2", "failed")],
        );

        assert_eq!(facts[0].1.transform_status.as_deref(), Some("failed"));
    }

    #[test]
    fn a_missing_measurement_stays_absent_rather_than_becoming_zero() {
        let row = SyncRow {
            connector: "alpha".to_owned(),
            claim: "out_of_band".to_owned(),
            status: "ok".to_owned(),
            started_at: Utc.timestamp_opt(0, 0).unwrap(),
            duration_ms: 1,
            records_moved: 400,
            rows_landed: 0,
            has_measurement: false,
        };

        assert_eq!(sync_facts(vec![row])[0].1.rows_landed, None);
    }

    #[test]
    fn a_measured_zero_is_reported_as_a_zero() {
        let row = SyncRow {
            connector: "alpha".to_owned(),
            claim: "claimed".to_owned(),
            status: "ok".to_owned(),
            started_at: Utc.timestamp_opt(0, 0).unwrap(),
            duration_ms: 1,
            records_moved: 400,
            rows_landed: 0,
            has_measurement: true,
        };

        assert_eq!(
            sync_facts(vec![row])[0].1.rows_landed,
            Some(0),
            "moved records and nothing landed is the finding, not a gap"
        );
    }

    #[test]
    fn every_statement_resolves_its_placeholders() {
        for template in [
            RUNS_SQL,
            TRANSFORMS_SQL,
            SYNCS_SQL,
            STORAGE_SQL,
            STREAMS_SQL,
            CONFIGURED_SQL,
            HISTORY_SQL,
        ] {
            let rendered = sql(template);
            assert!(
                !rendered.contains('{'),
                "unresolved placeholder in: {rendered}"
            );
            assert!(
                rendered.contains(LEDGER),
                "must read the ledger: {rendered}"
            );
        }
    }

    #[test]
    fn the_configured_set_is_keyed_on_a_sealed_snapshot() {
        assert!(
            sql(CONFIGURED_SQL).contains("sweep.completed"),
            "without the marker a half-written snapshot would read as the whole set"
        );
    }

    #[test]
    fn an_absent_ledger_is_told_apart_from_a_warehouse_fault() {
        let absent = clickhouse::error::Error::Custom("Code: 60. UNKNOWN_TABLE".to_owned());
        let fault = clickhouse::error::Error::Custom("Code: 241. MEMORY_LIMIT_EXCEEDED".to_owned());

        assert!(absent_ledger(&absent));
        assert!(!absent_ledger(&fault));
    }
}
