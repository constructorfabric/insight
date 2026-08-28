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

/// Connectors the summary will serve.
///
/// The set is bounded by the build's descriptor list in practice, so this is a
/// backstop rather than a page: an install cannot reach it by configuring more
/// connectors, only by accumulating names in the ledger that no build has. It
/// exists because an unbounded response is a bug however unlikely the input —
/// and because reaching it should be visible rather than silent, the read logs
/// when it truncates.
const CONNECTOR_LIMIT: u32 = 500;

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

/// Orders every recorded row of every sync, newest last. Never NULL.
///
/// Four components, each earning its place:
///
/// * `coalesce(job_updated_at, ts)` — the axis the ledger places jobs along,
///   falling back to when the row was recorded. The writer no longer leaves a
///   NULL here, but rows recorded before the placement rule settled still hold
///   one and a reader cannot survive it: ClickHouse sorts NULLs last in BOTH
///   directions, so such a row would not merely lose the comparison — a
///   different, older job would win it, and the page would present a stale
///   success as the current state.
/// * `toUInt64OrZero(job_id)` — the mover's ids are numbers stored as text, so
///   comparing them as text makes `"9"` newer than `"10"`.
/// * the terminal flag — within one job a final row outranks a provisional one
///   whatever the clocks say. Two rows of one job can share a millisecond, and
///   without this the answer is decided by physical row order.
/// * `ts` — the last resort, newest recorded wins.
static ROW_ORDER: LazyLock<String> = LazyLock::new(|| {
    let terminal = SyncStatus::TERMINAL
        .iter()
        .map(|status| format!("'{}'", status.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "(coalesce(job_updated_at, ts), toUInt64OrZero(job_id), \
          status IN ({terminal}), ts)"
    )
});

/// The columns a resolved sync carries, as one tuple.
///
/// INVARIANT: resolved as a single tuple, never column by column. `argMax`
/// ignores rows whose value argument is NULL, so six independent `argMax`
/// calls answer from six independently-chosen rows — one column taken from
/// the newest job and another from an older one whose value happened not to
/// be NULL, producing a row that never existed.
const SYNC_COLUMNS: &str = "job_id, status, started_at, job_updated_at, \
     duration_ms, records_reported";

/// The unpacked tuple, under names no ledger column carries.
///
/// A `resolved_` prefix rather than the column's own name: an alias equal to a
/// column name is harmless where nothing in scope carries that column, but a
/// guard that must know which of the two cases it is looking at is a guard
/// with an exception. This keeps the rule absolute.
const UNPACK_WINNER: &str = "winner.1 AS resolved_job_id, \
     winner.2 AS resolved_status, winner.3 AS resolved_started_at, \
     winner.4 AS resolved_job_updated_at, winner.5 AS resolved_duration_ms, \
     winner.6 AS resolved_records_reported";

/// The newest sync per connector.
///
/// An aggregate, not a sort. Sorting the relation by a column outside its sort
/// key reads and orders the whole retention window to answer with one row per
/// connector — measured at hundreds of megabytes where this stays at single
/// digits, and the service caps its own query memory.
static LAST_SYNC_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT connector, {UNPACK_WINNER} \
         FROM (SELECT connector, argMax(tuple({SYNC_COLUMNS}), ord) AS winner \
               FROM (SELECT connector, {SYNC_COLUMNS}, {order} AS ord \
                     FROM {TABLE} WHERE event = '{SYNC_COMPLETED}') \
               GROUP BY connector \
               ORDER BY connector LIMIT ?)",
        order = &*ROW_ORDER
    )
});

/// The set the controller managed on one sealed tick.
static CONFIGURED_SET_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT DISTINCT connector FROM {TABLE} \
         WHERE event = '{CONNECTOR_CONFIGURED}' AND tick_id = ?"
    )
});

/// One connector's recent syncs, one row per job, newest first.
///
/// Narrowed by the sort key's first two columns before anything is grouped, so
/// the aggregate sees one connector's rows rather than the whole relation.
static SYNC_HISTORY_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT {UNPACK_WINNER} \
         FROM (SELECT argMax(tuple({SYNC_COLUMNS}), ord) AS winner, \
                      max(ord) AS newest \
               FROM (SELECT {SYNC_COLUMNS}, {order} AS ord \
                     FROM {TABLE} \
                     WHERE event = '{SYNC_COMPLETED}' AND connector = ?) \
               GROUP BY job_id) \
         ORDER BY newest DESC LIMIT ?",
        order = &*ROW_ORDER
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
/// INVARIANT: every `Nullable` column is `Option` here. A non-`Option` field
/// against a nullable column is rejected while decoding the row, which yields a
/// 500 — and only once rows exist, so an empty ledger hides it completely.
///
/// The `resolved_` names are the SQL aliases, not the domain's: the statement
/// must not alias anything to a ledger column name, and the rename keeps that
/// constraint in the one place it belongs.
#[derive(Debug, Deserialize, clickhouse::Row)]
struct SyncRow {
    connector: String,
    #[serde(rename = "resolved_job_id")]
    job_id: String,
    #[serde(rename = "resolved_status")]
    status: String,
    #[serde(
        rename = "resolved_started_at",
        with = "clickhouse::serde::chrono::datetime64::millis::option"
    )]
    started_at: Option<DateTime<Utc>>,
    #[serde(
        rename = "resolved_job_updated_at",
        with = "clickhouse::serde::chrono::datetime64::millis::option"
    )]
    job_updated_at: Option<DateTime<Utc>>,
    #[serde(rename = "resolved_duration_ms")]
    duration_ms: Option<u64>,
    #[serde(rename = "resolved_records_reported")]
    records_reported: Option<u64>,
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct HistoryRow {
    #[serde(rename = "resolved_job_id")]
    job_id: String,
    #[serde(rename = "resolved_status")]
    status: String,
    #[serde(
        rename = "resolved_started_at",
        with = "clickhouse::serde::chrono::datetime64::millis::option"
    )]
    started_at: Option<DateTime<Utc>>,
    #[serde(
        rename = "resolved_job_updated_at",
        with = "clickhouse::serde::chrono::datetime64::millis::option"
    )]
    job_updated_at: Option<DateTime<Utc>>,
    #[serde(rename = "resolved_duration_ms")]
    duration_ms: Option<u64>,
    #[serde(rename = "resolved_records_reported")]
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
        ch.query(&LAST_SYNC_SQL)
            .bind(CONNECTOR_LIMIT)
            .fetch_all::<SyncRow>(),
        configured_set(ch, sealed.as_ref().map(|tick| tick.tick_id.as_str())),
    )?;

    // A sealed tick IS recorded history, even when it found no connector: the
    // mover was read, and the page must say when rather than claim nothing has
    // been read. Deriving this from the rows alone would make a sealed empty
    // install indistinguishable from one that has never recorded anything.
    if syncs.len() >= CONNECTOR_LIMIT as usize {
        tracing::warn!(
            limit = CONNECTOR_LIMIT,
            "connector health truncated the summary; the ledger holds at least \
             as many connector names as the response can carry"
        );
    }

    let has_history = sealed.is_some() || !syncs.is_empty();
    let summaries = merge(syncs, &configured);
    Ok(LedgerFacts {
        sealed_at: sealed.map(|tick| tick.sealed_at),
        typical_read_interval_ms: (interval.gaps >= MIN_GAPS_FOR_INTERVAL)
            .then_some(interval.interval_ms),
        has_history,
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
            job_updated_at: self.job_updated_at,
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
            job_updated_at: row.job_updated_at,
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

    /// The migration itself, not a copy of it: a column added there and not
    /// here would otherwise drop silently out of the guard below.
    const MIGRATION: &str = include_str!(
        "../../../../../../ingestion/scripts/migrations/20260827000000_connector-sync-history.sql"
    );

    /// Every SQL statement this module issues, so a new one joins the guards
    /// automatically rather than being remembered into them.
    fn statements() -> [(&'static str, &'static str); 5] {
        [
            ("SEALED_TICK_SQL", SEALED_TICK_SQL.as_str()),
            ("READ_INTERVAL_SQL", READ_INTERVAL_SQL.as_str()),
            ("LAST_SYNC_SQL", LAST_SYNC_SQL.as_str()),
            ("CONFIGURED_SET_SQL", CONFIGURED_SET_SQL.as_str()),
            ("SYNC_HISTORY_SQL", SYNC_HISTORY_SQL.as_str()),
        ]
    }

    /// The ledger's column names, read out of the `CREATE TABLE` block.
    fn ledger_columns() -> Vec<String> {
        let body = MIGRATION
            .split_once("CREATE TABLE")
            .and_then(|(_, tail)| tail.split_once('('))
            .map(|(_, tail)| tail)
            .unwrap_or_default();
        body.lines()
            .map(str::trim)
            .take_while(|line| !line.starts_with(')'))
            .filter_map(|line| line.split_whitespace().next())
            .filter(|name| name.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .map(str::to_owned)
            .collect()
    }

    /// Lower-cased, whitespace-collapsed, quoting stripped.
    ///
    /// Every one of these is invisible to a raw substring scan and shadows a
    /// column exactly the same way in ClickHouse: `AS  ts`, `as ts`, a newline
    /// between the two, and `AS "ts"`.
    fn normalised(sql: &str) -> String {
        sql.to_ascii_lowercase()
            .replace(['`', '"'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn the_column_list_comes_from_the_migration() {
        let columns = ledger_columns();
        for expected in ["event_id", "ts", "tick_id", "job_id", "connector", "event"] {
            assert!(
                columns.iter().any(|c| c == expected),
                "missing {expected}: {columns:?}"
            );
        }
        assert_eq!(columns.len(), 11, "{columns:?}");
    }

    /// An alias that repeats a column name shadows that column for every other
    /// expression in the same SELECT, so an aggregate reading it reads the alias
    /// instead and ClickHouse answers `ILLEGAL_AGGREGATION`.
    #[test]
    fn no_statement_aliases_a_value_to_a_ledger_column_name() {
        let columns = ledger_columns();
        for (name, sql) in statements() {
            let flat = normalised(sql);
            for column in &columns {
                assert!(
                    !flat.contains(&format!(" as {column}")),
                    "{name} aliases a value to `{column}`, a ledger column",
                );
                assert!(
                    !flat.contains(&format!(") {column}")),
                    "{name} implicitly aliases a value to `{column}`, a ledger column",
                );
            }
        }
    }

    #[test]
    fn the_alias_scan_catches_every_spelling_of_the_shadow() {
        let columns = ledger_columns();
        let shadows = [
            "select max(ts) AS ts from t",
            "select max(ts) AS  ts from t",
            "select max(ts) as ts from t",
            "select max(ts) ts from t",
            "select max(ts)\nAS ts from t",
            "select max(ts) AS\nts from t",
            "select max(ts) AS \"ts\" from t",
        ];
        for shadow in shadows {
            let flat = normalised(shadow);
            let caught = columns.iter().any(|column| {
                flat.contains(&format!(" as {column}")) || flat.contains(&format!(") {column}"))
            });
            assert!(caught, "the guard would miss: {shadow}");
        }

        let innocent = normalised("select ts as sealed_at, x as y from t");
        assert!(
            !columns
                .iter()
                .any(|c| innocent.contains(&format!(" as {c}"))),
            "the guard must not fire on an alias that is not a column name"
        );
    }

    #[test]
    fn every_statement_reads_only_the_ledger() {
        for (name, sql) in statements() {
            for tail in normalised(sql).split("from ").skip(1) {
                assert!(
                    tail.starts_with('(') || tail.starts_with(TABLE),
                    "{name} reads something other than the ledger \
                     (`from {}`); the page must answer from one relation (FR-11)",
                    tail.split_whitespace().next().unwrap_or(""),
                );
            }
        }
    }

    #[test]
    fn the_resolution_never_aggregates_a_column_on_its_own() {
        // Six independent `argMax` calls answer from six independently-chosen
        // rows, because `argMax` skips a NULL value argument per column. Both
        // resolutions must aggregate the tuple.
        for (name, sql) in [
            ("LAST_SYNC_SQL", LAST_SYNC_SQL.as_str()),
            ("SYNC_HISTORY_SQL", SYNC_HISTORY_SQL.as_str()),
        ] {
            let flat = normalised(sql);
            assert_eq!(
                flat.matches("argmax(").count(),
                1,
                "{name} must resolve one tuple, not one column at a time",
            );
            assert!(flat.contains("argmax(tuple("), "{name}");
        }
    }

    #[test]
    fn the_row_order_is_never_null() {
        // A NULL sort key does not lose a comparison in ClickHouse — it wins a
        // different one, because NULLs sort last in both directions.
        let order = normalised(&ROW_ORDER);
        assert!(order.contains("coalesce(job_updated_at, ts)"), "{order}");
        assert!(order.contains("touint64orzero(job_id)"), "{order}");
        assert!(order.contains("status in ("), "{order}");
    }

    #[test]
    fn the_terminal_vocabulary_is_rendered_from_one_source() {
        for status in SyncStatus::TERMINAL {
            assert!(
                ROW_ORDER.contains(status.as_str()),
                "the row order must know {} is terminal",
                status.as_str()
            );
        }
        assert!(
            !ROW_ORDER.contains(SyncStatus::Running.as_str()),
            "a provisional status must not read as terminal"
        );
    }

    #[test]
    fn every_row_returning_read_is_bounded() {
        // An unbounded response is a bug however unlikely the input, and the
        // repository's own rule is to bound it at the edge.
        for (name, sql) in [
            ("SYNC_HISTORY_SQL", SYNC_HISTORY_SQL.as_str()),
            ("LAST_SYNC_SQL", LAST_SYNC_SQL.as_str()),
            ("SEALED_TICK_SQL", SEALED_TICK_SQL.as_str()),
            ("READ_INTERVAL_SQL", READ_INTERVAL_SQL.as_str()),
        ] {
            assert!(
                normalised(sql).contains("limit "),
                "{name} can return an unbounded number of rows",
            );
        }
    }

    /// The configured set is the one read with no row limit, deliberately: it is
    /// filtered to a single tick, so its size is the number of connectors that
    /// tick managed — and dropping members of a snapshot would make it read as a
    /// smaller set than the one that was sealed.
    #[test]
    fn the_configured_set_is_bounded_by_its_tick_not_by_a_limit() {
        let sql = normalised(&CONFIGURED_SET_SQL);
        assert!(sql.contains("tick_id = ?"), "{sql}");
        assert!(!sql.contains("limit "), "{sql}");
    }
}
