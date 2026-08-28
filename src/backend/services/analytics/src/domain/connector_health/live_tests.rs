//! Live ClickHouse tests for the sync-ledger reads.
//!
//! These exist because an empty table hides every failure that matters here.
//! A `Nullable` column read into a non-`Option` field, an alias shadowing its
//! own source, a resolution that ties between two rows of one job — all of them
//! pass against zero rows and answer 500 the moment a sweep has run. So this
//! inserts rows and reads them back through the real query path.
//!
//! `#[ignore]`d and skipping silently when `INTEGRATION_TESTS_CLICKHOUSE_URL`
//! is unset (the convention the other live tests here use); optional auth via
//! `INTEGRATION_TESTS_CLICKHOUSE_USER` / `..._PASSWORD`.
//!
//! One test, run in sequence, on purpose: the newest sealed tick and the
//! per-connector summary are both instance-wide reads, so two of these running
//! in parallel against a shared server would read each other's rows.

#![expect(
    clippy::expect_used,
    reason = "a live-server setup failure should fail the test loudly"
)]

use chrono::{Duration, SecondsFormat, Utc};

use super::model::SyncStatus;
use super::read::{absent_ledger, read_health, read_syncs};

const URL_VAR: &str = "INTEGRATION_TESTS_CLICKHOUSE_URL";

/// The table this reads comes from the migration itself, not a copy of it: a
/// column added there and not here would otherwise pass every test and fail on
/// the first install to deploy both.
const MIGRATION: &str = include_str!(
    "../../../../../../ingestion/scripts/migrations/20260827000000_connector-sync-history.sql"
);

const TABLE: &str = "ingestion_history.sync_events";

/// Stamps are anchored to now, not to a fixed date.
///
/// The table's TTL runs from `ts`, so a row stamped further back than the
/// retention window is dropped the moment it lands — which reads exactly like a
/// query that returned nothing. Fixed dates in a test therefore rot silently
/// into a false failure the day retention passes them.
fn ago(minutes: i64) -> String {
    (Utc::now() - Duration::minutes(minutes))
        .to_rfc3339_opts(SecondsFormat::Millis, false)
        .replace('T', " ")
        .replace("+00:00", "")
}

// Empty counts as unset: the CI matrix passes '' to entries without a
// provisioned ClickHouse, and set-but-empty must skip exactly like absent.
fn client_or_skip() -> Option<insight_clickhouse::Client> {
    let url = std::env::var(URL_VAR).unwrap_or_default();
    if url.is_empty() {
        eprintln!("skipping: {URL_VAR} not set");
        return None;
    }
    let mut config = insight_clickhouse::Config::new(url, "default");
    if let (Ok(user), Ok(password)) = (
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_USER"),
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_PASSWORD"),
    ) && !user.is_empty()
    {
        config = config.with_auth(user, password);
    }
    Some(insight_clickhouse::Client::new(config))
}

/// The HTTP interface runs one statement per request, so the migration is
/// fanned out the way `lib/ch-exec.sh` fans it out: drop whole-line comments,
/// split on `;`.
async fn apply_migration(ch: &insight_clickhouse::Client) {
    let body: String = MIGRATION
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    for statement in body.split(';') {
        if statement.trim().is_empty() {
            continue;
        }
        ch.query(statement)
            .execute()
            .await
            .unwrap_or_else(|error| panic!("migration statement failed: {error}\n{statement}"));
    }
}

async fn truncate(ch: &insight_clickhouse::Client) {
    ch.query(&format!("TRUNCATE TABLE {TABLE}"))
        .execute()
        .await
        .expect("truncate");
}

/// One row, written the way the sweep writes it — every column named, so a
/// column this test forgets is a compile-time-invisible but run-time-loud
/// mismatch rather than a silent DEFAULT.
struct Row<'a> {
    /// Set explicitly rather than left to the column's DEFAULT: three inserts
    /// in quick succession can share a millisecond, which would make the gap
    /// between seals zero and the interval assertion below flaky.
    ts: &'a str,
    tick_id: &'a str,
    job_id: &'a str,
    connector: &'a str,
    event: &'a str,
    status: &'a str,
    started_at: Option<&'a str>,
    job_updated_at: Option<&'a str>,
    duration_ms: Option<u64>,
    records_reported: Option<u64>,
}

impl Row<'_> {
    fn values(&self) -> String {
        let moment = |value: Option<&str>| match value {
            Some(stamp) => format!("toDateTime64('{stamp}', 3, 'UTC')"),
            None => "NULL".to_owned(),
        };
        let number = |value: Option<u64>| match value {
            Some(count) => count.to_string(),
            None => "NULL".to_owned(),
        };
        format!(
            "(toDateTime64('{}', 3, 'UTC'), '{}', '{}', '{}', '{}', '{}', {}, {}, {}, {})",
            self.ts,
            self.tick_id,
            self.job_id,
            self.connector,
            self.event,
            self.status,
            moment(self.started_at),
            moment(self.job_updated_at),
            number(self.duration_ms),
            number(self.records_reported),
        )
    }
}

async fn insert(ch: &insight_clickhouse::Client, rows: &[Row<'_>]) {
    let values: Vec<String> = rows.iter().map(Row::values).collect();
    let sql = format!(
        "INSERT INTO {TABLE} (ts, tick_id, job_id, connector, event, status, \
         started_at, job_updated_at, duration_ms, records_reported) VALUES {}",
        values.join(", ")
    );
    ch.query(&sql).execute().await.expect("insert");
}

fn sync<'a>(job_id: &'a str, connector: &'a str, status: &'a str, updated: &'a str) -> Row<'a> {
    Row {
        ts: updated,
        tick_id: "tick-1",
        job_id,
        connector,
        event: "sync.completed",
        status,
        started_at: Some(updated),
        job_updated_at: Some(updated),
        duration_ms: Some(1_000),
        records_reported: Some(7),
    }
}

fn configured<'a>(connector: &'a str, tick_id: &'a str, at: &'a str) -> Row<'a> {
    Row {
        ts: at,
        tick_id,
        job_id: "",
        connector,
        event: "connector.configured",
        status: "",
        started_at: None,
        job_updated_at: None,
        duration_ms: None,
        records_reported: None,
    }
}

fn seal<'a>(tick_id: &'a str, at: &'a str) -> Row<'a> {
    Row {
        ts: at,
        tick_id,
        job_id: "",
        connector: "",
        event: "sweep.completed",
        status: "",
        started_at: None,
        job_updated_at: None,
        duration_ms: None,
        records_reported: None,
    }
}

/// Three ticks a quarter of an hour apart, and three jobs beneath them.
struct Stamps {
    tick_1: String,
    tick_2: String,
    tick_3: String,
    job_oldest: String,
    job_older: String,
    job_newer: String,
}

impl Stamps {
    fn fresh() -> Self {
        Self {
            tick_1: ago(40),
            tick_2: ago(25),
            tick_3: ago(10),
            job_oldest: ago(70),
            job_older: ago(60),
            job_newer: ago(45),
        }
    }
}

#[tokio::test]
#[ignore = "needs a live ClickHouse"]
async fn the_ledger_reads_answer_from_real_rows() {
    let Some(ch) = client_or_skip() else { return };
    apply_migration(&ch).await;
    let at = Stamps::fresh();

    an_empty_ledger_says_so(&ch).await;
    a_sealed_empty_sweep_still_counts_as_history(&ch, &at).await;
    truncate(&ch).await;
    rows_cross_into_options(&ch, &at).await;
    two_rows_for_one_job_resolve_to_the_newer(&ch, &at).await;
    a_dropped_connector_stops_reading_configured(&ch, &at).await;
    one_connectors_window_is_newest_first(&ch).await;

    truncate(&ch).await;
    the_resolution_survives_every_way_it_used_to_be_wrong(&ch, &at).await;
}

/// Three answers the previous shape got wrong, each measured against a real
/// server before the shape was replaced.
///
/// It sorted the relation and read the winner off the top, which meant: job ids
/// compared as text so `"9"` beat `"10"`; a NULL update stamp sorting last in
/// BOTH directions so an older job won outright; and two rows of one job
/// sharing a millisecond decided by physical row order. Each produced a
/// confident wrong answer rather than an absence.
async fn the_resolution_survives_every_way_it_used_to_be_wrong(
    ch: &insight_clickhouse::Client,
    at: &Stamps,
) {
    insert(
        ch,
        &[
            // Same update moment, ids 9 and 10: text order says 9 is newer.
            sync("9", "numeric", "succeeded", &at.job_older),
            Row {
                ts: &at.tick_1,
                ..sync("10", "numeric", "running", &at.job_older)
            },
            // The same job's terminal row, written in the SAME millisecond as
            // its provisional one.
            Row {
                ts: &at.tick_1,
                records_reported: Some(0),
                ..sync("10", "numeric", "failed", &at.job_older)
            },
            // A newer job the mover gave no update stamp for.
            sync("1", "unplaced", "succeeded", &at.job_older),
            Row {
                ts: &at.tick_3,
                job_updated_at: None,
                ..sync("2", "unplaced", "failed", &at.job_newer)
            },
        ],
    )
    .await;

    let facts = read_health(ch).await.expect("read");

    let numeric = summary_for(&facts.summaries, "numeric");
    let resolved = numeric.last_sync.as_ref().expect("numeric has synced");
    assert_eq!(
        resolved.job_id, "10",
        "job ids are numbers: compared as text, `9` outranks `10`"
    );
    assert_eq!(
        resolved.status,
        SyncStatus::Failed,
        "a terminal row outranks a provisional one written in the same millisecond"
    );
    assert_eq!(
        resolved.records_reported,
        Some(0),
        "the terminal row's own reported zero, not the provisional row's absence"
    );

    let unplaced = summary_for(&facts.summaries, "unplaced");
    let resolved = unplaced.last_sync.as_ref().expect("unplaced has synced");
    assert_eq!(
        resolved.job_id, "2",
        "a job with no update stamp must not hand the answer to an older one"
    );
    assert!(
        resolved.job_updated_at.is_none(),
        "the resolved row must be one real row, not a mix of two: job 2 has no \
         update stamp, so this must be absent rather than job 1's stamp"
    );
    assert_eq!(resolved.status, SyncStatus::Failed);
}

/// A sweep can seal having found no connector — a fresh install before any is
/// configured. The mover WAS read then, so the page must date itself rather than
/// say nothing has been read.
async fn a_sealed_empty_sweep_still_counts_as_history(
    ch: &insight_clickhouse::Client,
    at: &Stamps,
) {
    truncate(ch).await;
    insert(ch, &[seal("tick-empty", &at.tick_1)]).await;

    let facts = read_health(ch).await.expect("read");
    assert!(
        facts.has_history,
        "a sealed tick is recorded history even with no connector in it"
    );
    assert!(facts.sealed_at.is_some());
    assert!(facts.summaries.is_empty());
}

async fn an_empty_ledger_says_so(ch: &insight_clickhouse::Client) {
    truncate(ch).await;
    let empty = read_health(ch).await.expect("empty ledger must not error");
    assert!(!empty.has_history, "nothing recorded means no history");
    assert!(empty.sealed_at.is_none());
    assert!(empty.summaries.is_empty());
    assert!(
        empty.typical_read_interval_ms.is_none(),
        "no ticks means no measured interval, not a zero"
    );
}

async fn rows_cross_into_options(ch: &insight_clickhouse::Client, at: &Stamps) {
    insert(
        ch,
        &[
            sync("1", "alpha", "succeeded", &at.job_older),
            Row {
                started_at: None,
                duration_ms: None,
                records_reported: None,
                ..sync("2", "alpha", "running", &at.job_newer)
            },
            sync("3", "bravo", "failed", &at.job_oldest),
            configured("alpha", "tick-1", &at.tick_1),
            configured("bravo", "tick-1", &at.tick_1),
            seal("tick-1", &at.tick_1),
        ],
    )
    .await;

    let facts = read_health(ch).await.expect("rows must not break the read");
    assert!(facts.has_history);
    assert!(facts.sealed_at.is_some(), "the sealed tick dates the facts");
    assert_eq!(facts.summaries.len(), 2, "one row per connector");

    let alpha = summary_for(&facts.summaries, "alpha");
    let alpha_sync = alpha.last_sync.as_ref().expect("alpha has synced");
    assert_eq!(alpha_sync.job_id, "2", "the newest job wins");
    assert_eq!(alpha_sync.status, SyncStatus::Running);
    assert!(
        alpha_sync.started_at.is_none(),
        "a job not started has no start, not the epoch"
    );
    assert!(alpha_sync.duration_ms.is_none());
    assert!(alpha_sync.records_reported.is_none());
    assert!(alpha.configured);

    assert_eq!(
        facts.summaries[0].connector, "bravo",
        "a failed sync outranks a running one"
    );
}

async fn two_rows_for_one_job_resolve_to_the_newer(ch: &insight_clickhouse::Client, at: &Stamps) {
    insert(
        ch,
        &[Row {
            ts: &at.tick_2,
            tick_id: "tick-2",
            records_reported: Some(0),
            ..sync("2", "alpha", "failed", &at.job_newer)
        }],
    )
    .await;

    let facts = read_health(ch).await.expect("read");
    let alpha = summary_for(&facts.summaries, "alpha");
    let alpha_sync = alpha.last_sync.as_ref().expect("alpha has synced");
    assert_eq!(
        alpha_sync.status,
        SyncStatus::Failed,
        "the newest row of a job is its account, not the first one seen"
    );
    assert_eq!(
        alpha_sync.records_reported,
        Some(0),
        "a reported zero is a measurement"
    );
}

async fn a_dropped_connector_stops_reading_configured(
    ch: &insight_clickhouse::Client,
    at: &Stamps,
) {
    // Two further seals a known distance apart, so the interval below is a fact
    // this test states rather than one it reads back out of the query.
    insert(
        ch,
        &[
            seal("tick-2", &at.tick_2),
            configured("alpha", "tick-3", &at.tick_3),
            seal("tick-3", &at.tick_3),
        ],
    )
    .await;

    let facts = read_health(ch).await.expect("read");
    let bravo = summary_for(&facts.summaries, "bravo");
    assert!(
        !bravo.configured,
        "dropped from the snapshot means no longer configured"
    );
    assert_eq!(
        facts.summaries.last().map(|row| row.connector.as_str()),
        Some("bravo"),
        "a removed connector sorts last, not first"
    );
    assert_eq!(
        facts.typical_read_interval_ms,
        Some(15 * 60 * 1_000),
        "three seals fifteen minutes apart make the measured interval fifteen \
         minutes — measured, not read from a chart value"
    );
}

async fn one_connectors_window_is_newest_first(ch: &insight_clickhouse::Client) {
    let history = read_syncs(ch, "alpha").await.expect("history");
    assert_eq!(history.len(), 2, "one row per job, not per recorded row");
    assert_eq!(history[0].job_id, "2", "newest first");
    assert_eq!(history[1].job_id, "1");

    let missing = read_syncs(ch, "nobody").await.expect("no such connector");
    assert!(
        missing.is_empty(),
        "an unknown connector is empty, not an error"
    );
}

fn summary_for<'a>(
    summaries: &'a [super::model::ConnectorSummary],
    connector: &str,
) -> &'a super::model::ConnectorSummary {
    summaries
        .iter()
        .find(|row| row.connector == connector)
        .unwrap_or_else(|| panic!("no summary for {connector}"))
}

#[tokio::test]
#[ignore = "needs a live ClickHouse"]
async fn a_missing_relation_is_recognised_by_its_real_error() {
    let Some(ch) = client_or_skip() else { return };

    let error = ch
        .query("SELECT 1 FROM ingestion_history.no_such_table")
        .fetch_all::<u8>()
        .await
        .expect_err("reading a table that is not there must fail");

    assert!(
        absent_ledger(&error),
        "the absent-relation classifier must recognise what ClickHouse \
         actually says, not what it was assumed to say: {error}"
    );
}
