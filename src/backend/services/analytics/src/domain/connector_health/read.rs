//! Reading the sync ledger. Every statement here touches one relation.
//!
//! The ledger is written by the reconcile loop and read here; the two never
//! meet. That is what lets this page answer while the mover, the cluster API
//! and the ingestion pipeline are all unreachable.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use super::model::{ConnectorSummary, LastSync, LedgerFacts, SyncStatus};

/// DDL owned by `scripts/migrations/20260827000000_connector-sync-history.sql`;
/// the query-path role holds `SELECT` here and nothing that writes.
const TABLE: &str = "ingestion_history.sync_events";

const SYNC_COMPLETED: &str = "sync.completed";
const CONNECTOR_CONFIGURED: &str = "connector.configured";
const SWEEP_COMPLETED: &str = "sweep.completed";

/// Rows in a connector's expandable history. A window, not the retention.
pub(crate) const HISTORY_WINDOW: u32 = 50;

/// Sealed ticks sampled for the median gap between reads. Enough to survive one
/// unusual interval; short enough that a cadence change shows up quickly.
const INTERVAL_SAMPLE: u32 = 20;

/// Two gaps is the fewest that has a median worth reporting.
const MIN_GAPS_FOR_INTERVAL: u64 = 2;

/// The newest tick that finished writing.
///
/// INVARIANT: the timestamp is aliased `sealed_at`, never `ts`. An alias that
/// shadows its own source column is read by every other expression in the same
/// SELECT — including the aggregate that needs the column — and ClickHouse
/// answers `ILLEGAL_AGGREGATION` rather than a wrong number. The guard test
/// below fails if a future edit reintroduces the shadow.
static SEALED_TICK_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT tick_id, ts AS sealed_at FROM {TABLE} \
         WHERE event = '{SWEEP_COMPLETED}' ORDER BY ts DESC LIMIT 1"
    )
});

/// How often the mover has actually been read lately.
///
/// `arrayDifference` over the sorted stamps yields the gaps with a leading
/// zero, which the filter drops along with any two seals inside the same
/// millisecond. Measured rather than configured: reading an intended cadence
/// from chart values would let the page assert a schedule it cannot verify.
static READ_INTERVAL_SQL: LazyLock<String> = LazyLock::new(|| {
    // SAFETY: `median` of an empty array is NaN, and `toUInt64(NaN)` is
    // CANNOT_CONVERT_TYPE — a 500 on every install whose ledger is empty, which
    // is every install before its first sweep. `ifNull` does not catch it: NaN
    // is a value, not a null. So the length is checked before the median is
    // taken, and the gap count is reported separately for the caller to judge.
    format!(
        "SELECT toUInt64(if(length(gap_list) > 0, arrayReduce('median', gap_list), 0)) \
             AS interval_ms, \
           toUInt64(length(gap_list)) AS gaps \
         FROM (SELECT arrayFilter(x -> x > 0, \
                        arrayDifference(arraySort(groupArray(seal_ms)))) AS gap_list \
               FROM (SELECT toUnixTimestamp64Milli(ts) AS seal_ms FROM {TABLE} \
                     WHERE event = '{SWEEP_COMPLETED}' ORDER BY ts DESC LIMIT ?))"
    )
});

/// The newest sync per connector, resolved in two steps.
///
/// The inner step takes the newest ROW per job: a job seen in flight and later
/// seen finished has two rows sharing one creation time, so resolving straight
/// to the newest job would tie between them and could answer `running` for a
/// sync that ended. The outer step then takes the newest JOB per connector.
///
/// Both steps use `LIMIT 1 BY` rather than `argMax`: `argMax` ignores rows
/// whose value argument is NULL, so a sync row without a creation time would
/// take its whole connector out of the answer instead of merely losing the
/// comparison.
static LAST_SYNC_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT connector, job_id, status, started_at, job_created_at, \
           duration_ms, records_reported \
         FROM (SELECT connector, job_id, status, started_at, job_created_at, \
                      duration_ms, records_reported \
               FROM {TABLE} WHERE event = '{SYNC_COMPLETED}' \
               ORDER BY job_id, ts DESC LIMIT 1 BY job_id) \
         ORDER BY connector, job_created_at DESC, job_id DESC LIMIT 1 BY connector"
    )
});

/// The set the controller managed on one sealed tick.
static CONFIGURED_SET_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT DISTINCT connector FROM {TABLE} \
         WHERE event = '{CONNECTOR_CONFIGURED}' AND tick_id = ?"
    )
});

/// One connector's recent syncs, newest first, one row per job.
static SYNC_HISTORY_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT job_id, status, started_at, job_created_at, duration_ms, \
           records_reported \
         FROM (SELECT job_id, status, started_at, job_created_at, duration_ms, \
                      records_reported \
               FROM {TABLE} WHERE event = '{SYNC_COMPLETED}' AND connector = ? \
               ORDER BY job_id, ts DESC LIMIT 1 BY job_id) \
         ORDER BY job_created_at DESC, job_id DESC LIMIT ?"
    )
});

/// `UNKNOWN_TABLE` and `UNKNOWN_DATABASE`. The ledger is absent on an install
/// whose migration has not run and on a stand where nothing records, and the
/// page must say "nothing has been read yet" there rather than fail.
///
/// SAFETY: the trailing period is load-bearing. Without it `Code: 60` also
/// prefixes `Code: 600`, `Code: 601` and so on — so a future ClickHouse error
/// in that range would be classified as an absent ledger and the page would
/// serve an empty list during a real failure. The live test pins that this is
/// the shape ClickHouse actually produces.
const ABSENT_RELATION_CODES: [&str; 2] = ["Code: 60.", "Code: 81."];

#[derive(Debug, Deserialize, clickhouse::Row)]
struct SealedTickRow {
    tick_id: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    sealed_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct IntervalRow {
    interval_ms: u64,
    gaps: u64,
}

/// INVARIANT: every `Nullable` column is `Option` here. A non-`Option` field
/// against a nullable column is rejected while decoding the row, which yields a
/// 500 — and only once rows exist, so an empty ledger hides it completely.
#[derive(Debug, Deserialize, clickhouse::Row)]
struct SyncRow {
    connector: String,
    job_id: String,
    status: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    started_at: Option<DateTime<Utc>>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    job_created_at: Option<DateTime<Utc>>,
    duration_ms: Option<u64>,
    records_reported: Option<u64>,
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct HistoryRow {
    job_id: String,
    status: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    started_at: Option<DateTime<Utc>>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    job_created_at: Option<DateTime<Utc>>,
    duration_ms: Option<u64>,
    records_reported: Option<u64>,
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct ConnectorRow {
    connector: String,
}

pub(crate) async fn read_health(
    ch: &insight_clickhouse::Client,
) -> Result<LedgerFacts, clickhouse::error::Error> {
    let sealed = match ch
        .query(&SEALED_TICK_SQL)
        .fetch_optional::<SealedTickRow>()
        .await
    {
        Ok(row) => row,
        Err(error) if absent_ledger(&error) => return Ok(LedgerFacts::default()),
        Err(error) => return Err(error),
    };

    let (interval, syncs, configured) = tokio::try_join!(
        ch.query(&READ_INTERVAL_SQL)
            .bind(INTERVAL_SAMPLE)
            .fetch_one::<IntervalRow>(),
        ch.query(&LAST_SYNC_SQL).fetch_all::<SyncRow>(),
        configured_set(ch, sealed.as_ref().map(|tick| tick.tick_id.as_str())),
    )?;

    let summaries = merge(syncs, &configured);
    Ok(LedgerFacts {
        sealed_at: sealed.map(|tick| tick.sealed_at),
        typical_read_interval_ms: (interval.gaps >= MIN_GAPS_FOR_INTERVAL)
            .then_some(interval.interval_ms),
        has_history: !summaries.is_empty() || !configured.is_empty(),
        summaries,
    })
}

/// Bound to the tick resolved once above, never re-resolved per statement: a
/// sweep landing between two reads would otherwise answer with the newer
/// configured set beside the older facts — a state that existed on no tick.
async fn configured_set(
    ch: &insight_clickhouse::Client,
    tick_id: Option<&str>,
) -> Result<HashSet<String>, clickhouse::error::Error> {
    let Some(tick_id) = tick_id else {
        return Ok(HashSet::new());
    };
    let rows = ch
        .query(&CONFIGURED_SET_SQL)
        .bind(tick_id)
        .fetch_all::<ConnectorRow>()
        .await?;
    Ok(rows.into_iter().map(|row| row.connector).collect())
}

pub(crate) async fn read_syncs(
    ch: &insight_clickhouse::Client,
    connector: &str,
) -> Result<Vec<LastSync>, clickhouse::error::Error> {
    let rows = match ch
        .query(&SYNC_HISTORY_SQL)
        .bind(connector)
        .bind(HISTORY_WINDOW)
        .fetch_all::<HistoryRow>()
        .await
    {
        Ok(rows) => rows,
        Err(error) if absent_ledger(&error) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(rows.into_iter().map(HistoryRow::into_sync).collect())
}

impl HistoryRow {
    fn into_sync(self) -> LastSync {
        LastSync {
            job_id: self.job_id,
            status: SyncStatus::parse(&self.status),
            started_at: self.started_at,
            job_created_at: self.job_created_at,
            duration_ms: self.duration_ms,
            records_reported: self.records_reported,
        }
    }
}

/// The union of what synced and what is configured.
///
/// A connector that was never configured and never synced appears in neither,
/// so it cannot be listed — the reader may read this one relation and nothing
/// else, and no record of such a connector exists in it.
fn merge(syncs: Vec<SyncRow>, configured: &HashSet<String>) -> Vec<ConnectorSummary> {
    let mut by_connector: HashMap<String, Option<LastSync>> = configured
        .iter()
        .map(|connector| (connector.clone(), None))
        .collect();

    for row in syncs {
        let sync = LastSync {
            job_id: row.job_id,
            status: SyncStatus::parse(&row.status),
            started_at: row.started_at,
            job_created_at: row.job_created_at,
            duration_ms: row.duration_ms,
            records_reported: row.records_reported,
        };
        by_connector.insert(row.connector, Some(sync));
    }

    let mut summaries: Vec<ConnectorSummary> = by_connector
        .into_iter()
        .map(|(connector, last_sync)| ConnectorSummary {
            configured: configured.contains(&connector),
            connector,
            last_sync,
        })
        .collect();
    super::model::by_attention(&mut summaries);
    summaries
}

pub(super) fn absent_ledger(error: &clickhouse::error::Error) -> bool {
    let clickhouse::error::Error::BadResponse(payload) = error else {
        return false;
    };
    ABSENT_RELATION_CODES
        .iter()
        .any(|code| payload.trim_start().starts_with(code))
}

#[cfg(test)]
mod guards {
    use super::*;

    /// Every SQL constant here, so a new one joins the guards automatically
    /// rather than being remembered into them.
    fn statements() -> [(&'static str, &'static str); 5] {
        [
            ("SEALED_TICK_SQL", SEALED_TICK_SQL.as_str()),
            ("READ_INTERVAL_SQL", READ_INTERVAL_SQL.as_str()),
            ("LAST_SYNC_SQL", LAST_SYNC_SQL.as_str()),
            ("CONFIGURED_SET_SQL", CONFIGURED_SET_SQL.as_str()),
            ("SYNC_HISTORY_SQL", SYNC_HISTORY_SQL.as_str()),
        ]
    }

    /// Every column the ledger holds. An alias colliding with one of these is
    /// the bug this guards: the alias shadows the column for every other
    /// expression in the same SELECT, so an aggregate reading that column reads
    /// the alias instead and ClickHouse answers `ILLEGAL_AGGREGATION`.
    const LEDGER_COLUMNS: [&str; 11] = [
        "event_id",
        "ts",
        "tick_id",
        "job_id",
        "connector",
        "event",
        "status",
        "started_at",
        "job_created_at",
        "duration_ms",
        "records_reported",
    ];

    fn aliases(sql: &str) -> Vec<&str> {
        sql.split(" AS ")
            .skip(1)
            .filter_map(|tail| {
                tail.split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
            })
            .filter(|alias| !alias.is_empty())
            .collect()
    }

    #[test]
    fn no_statement_aliases_a_value_to_a_ledger_column_name() {
        for (name, sql) in statements() {
            for alias in aliases(sql) {
                assert!(
                    !LEDGER_COLUMNS.contains(&alias),
                    "{name} aliases a value to `{alias}`, which is a column of \
                     the ledger: the alias shadows the column for every other \
                     expression in the same SELECT",
                );
            }
        }
    }

    #[test]
    fn the_alias_scan_finds_what_it_is_meant_to_find() {
        assert_eq!(aliases("SELECT max(ts) AS ts FROM t"), vec!["ts"]);
        assert_eq!(
            aliases("SELECT ts AS sealed_at, x AS y"),
            vec!["sealed_at", "y"]
        );
        assert!(aliases("SELECT ts FROM t").is_empty());
    }

    #[test]
    fn every_statement_reads_only_the_ledger() {
        for (name, sql) in statements() {
            for tail in sql.split("FROM ").skip(1) {
                assert!(
                    tail.starts_with('(') || tail.starts_with(TABLE),
                    "{name} reads something other than the ledger \
                     (`FROM {}`); the page must answer from one relation (FR-11)",
                    tail.split_whitespace().next().unwrap_or(""),
                );
            }
        }
    }

    /// Both codes ClickHouse answers when the ledger is not there, verbatim.
    /// The live test pins that these are what it actually says; this pins that
    /// the classifier reads them and nothing else.
    #[test]
    fn an_absent_relation_is_classified_and_a_real_failure_is_not() {
        let absent = [
            "Code: 60. DB::Exception: Table ingestion_history.sync_events does not exist",
            "Code: 81. DB::Exception: Database ingestion_history does not exist",
            "  Code: 60. DB::Exception: leading whitespace still counts",
        ];
        for payload in absent {
            let error = clickhouse::error::Error::BadResponse(payload.to_owned());
            assert!(absent_ledger(&error), "should classify: {payload}");
        }

        let real = [
            "Code: 241. DB::Exception: Memory limit exceeded",
            "Code: 497. DB::Exception: Not enough privileges",
            "Code: 600. DB::Exception: a code merely starting with 60",
        ];
        for payload in real {
            let error = clickhouse::error::Error::BadResponse(payload.to_owned());
            assert!(
                !absent_ledger(&error),
                "an unreadable ledger must not be reported as an empty one: {payload}"
            );
        }
    }

    #[test]
    fn a_transport_failure_is_never_an_absent_relation() {
        // Serving an empty page because the network dropped would report "no
        // connector has ever synced" during exactly the outage that matters.
        let error = clickhouse::error::Error::Custom("connection reset".to_owned());
        assert!(!absent_ledger(&error));
    }

    #[test]
    fn history_is_a_bounded_window() {
        assert!(
            SYNC_HISTORY_SQL.contains("LIMIT ?"),
            "the window must be bound"
        );
    }
}
