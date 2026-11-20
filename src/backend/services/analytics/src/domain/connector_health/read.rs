use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use insight_clickhouse::MissingRelation;
use serde::Deserialize;

use super::model::{
    AppliedSchedule, ConnectorSummary, InstanceKey, LastSync, LedgerFacts, SyncStatus,
};
use super::name::{SourceId, TenantId};

const TABLE: &str = "ingestion_history.sync_events";

const SYNC_COMPLETED: &str = "sync.completed";
const CONNECTOR_CONFIGURED: &str = "connector.configured";
const SWEEP_COMPLETED: &str = "sweep.completed";

pub(crate) const HISTORY_WINDOW: u32 = 50;

const CONNECTOR_LIMIT: u32 = 500;

const INTERVAL_SAMPLE: u32 = 20;

const MIN_GAPS_FOR_INTERVAL: u64 = 2;

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

/// `arrayDifference` over the sorted stamps yields the gaps with a leading
/// zero, which the filter drops along with any two seals inside the same
/// millisecond.
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

/// INVARIANT: resolved as a single tuple, never column by column. `argMax`
/// ignores rows whose value argument is NULL, so six independent `argMax`
/// calls answer from six independently-chosen rows — one column taken from
/// the newest job and another from an older one whose value happened not to
/// be NULL, producing a row that never existed.
const SYNC_COLUMNS: &str = "job_id, status, started_at, job_updated_at, \
     duration_ms, records_reported";

const UNPACK_WINNER: &str = "winner.1 AS resolved_job_id, \
     winner.2 AS resolved_status, winner.3 AS resolved_started_at, \
     winner.4 AS resolved_job_updated_at, winner.5 AS resolved_duration_ms, \
     winner.6 AS resolved_records_reported";

const INSTANCE_COLUMNS: &str = "connector, tenant_id, source_id";

static LAST_SYNC_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT {INSTANCE_COLUMNS}, {UNPACK_WINNER} \
         FROM (SELECT {INSTANCE_COLUMNS}, argMax(tuple({SYNC_COLUMNS}), ord) AS winner \
               FROM (SELECT {INSTANCE_COLUMNS}, {SYNC_COLUMNS}, {order} AS ord \
                     FROM {TABLE} WHERE event = '{SYNC_COMPLETED}') \
               GROUP BY {INSTANCE_COLUMNS} \
               ORDER BY {INSTANCE_COLUMNS} LIMIT ?)",
        order = *ROW_ORDER
    )
});

static CONFIGURED_SET_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT DISTINCT {INSTANCE_COLUMNS}, schedules, schedule_timezone, schedule_origin, schedule_suspended \
         FROM {TABLE} \
         WHERE event = '{CONNECTOR_CONFIGURED}' AND tick_id = ?"
    )
});

static SYNC_HISTORY_SQL: LazyLock<String> = LazyLock::new(|| sync_history(""));

static INSTANCE_HISTORY_SQL: LazyLock<String> =
    LazyLock::new(|| sync_history(" AND tenant_id = ? AND source_id = ?"));

fn sync_history(scope: &str) -> String {
    format!(
        "SELECT {UNPACK_WINNER} \
         FROM (SELECT argMax(tuple({SYNC_COLUMNS}), ord) AS winner, \
                      max(ord) AS newest \
               FROM (SELECT {SYNC_COLUMNS}, {order} AS ord \
                     FROM {TABLE} \
                     WHERE event = '{SYNC_COMPLETED}' AND connector = ?{scope}) \
               GROUP BY job_id) \
         ORDER BY newest DESC LIMIT ?",
        order = *ROW_ORDER
    )
}

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
    tenant_id: String,
    source_id: String,
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
    tenant_id: String,
    source_id: String,
    schedules: Vec<String>,
    schedule_timezone: String,
    schedule_origin: String,
    schedule_suspended: bool,
}

impl ConnectorRow {
    fn into_entry(self) -> (InstanceKey, Option<AppliedSchedule>) {
        let key = InstanceKey {
            connector: self.connector,
            tenant_id: self.tenant_id,
            source_id: self.source_id,
        };
        let schedule = AppliedSchedule::from_record(
            self.schedules,
            self.schedule_timezone,
            &self.schedule_origin,
            self.schedule_suspended,
        );
        (key, schedule)
    }
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

    let has_history = sealed.is_some() || !syncs.is_empty();
    let summaries = merge(syncs, configured);
    Ok(LedgerFacts {
        sealed_at: sealed.map(|tick| tick.sealed_at),
        typical_read_interval_ms: (interval.gaps >= MIN_GAPS_FOR_INTERVAL)
            .then_some(interval.interval_ms),
        has_history,
        summaries,
    })
}

async fn configured_set(
    ch: &insight_clickhouse::Client,
    tick_id: Option<&str>,
) -> Result<HashMap<InstanceKey, Option<AppliedSchedule>>, clickhouse::error::Error> {
    let Some(tick_id) = tick_id else {
        return Ok(HashMap::new());
    };
    let rows = ch
        .query(&CONFIGURED_SET_SQL)
        .bind(tick_id)
        .fetch_all::<ConnectorRow>()
        .await?;
    Ok(rows.into_iter().map(ConnectorRow::into_entry).collect())
}

pub(crate) async fn read_syncs(
    ch: &insight_clickhouse::Client,
    connector: &str,
    scope: Option<&(TenantId, SourceId)>,
) -> Result<Vec<LastSync>, clickhouse::error::Error> {
    let query = match scope {
        Some((tenant_id, source_id)) => ch
            .query(&INSTANCE_HISTORY_SQL)
            .bind(connector)
            .bind(tenant_id.as_str())
            .bind(source_id.as_str()),
        None => ch.query(&SYNC_HISTORY_SQL).bind(connector),
    };
    let rows = match query.bind(HISTORY_WINDOW).fetch_all::<HistoryRow>().await {
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

fn merge(
    syncs: Vec<SyncRow>,
    mut configured: HashMap<InstanceKey, Option<AppliedSchedule>>,
) -> Vec<ConnectorSummary> {
    let mut by_instance: HashMap<InstanceKey, Option<LastSync>> = configured
        .keys()
        .map(|instance| (instance.clone(), None))
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
        let key = InstanceKey {
            connector: row.connector,
            tenant_id: row.tenant_id,
            source_id: row.source_id,
        };
        by_instance.insert(key, Some(sync));
    }

    let mut summaries: Vec<ConnectorSummary> = by_instance
        .into_iter()
        .map(|(instance, last_sync)| {
            let recorded = configured.remove(&instance);
            ConnectorSummary {
                configured: recorded.is_some(),
                schedule: recorded.flatten(),
                instance,
                last_sync,
            }
        })
        .collect();
    super::model::by_attention(&mut summaries);
    if summaries.len() > CONNECTOR_LIMIT as usize {
        tracing::warn!(
            limit = CONNECTOR_LIMIT,
            held = summaries.len(),
            "connector health truncated the summary; the ledger holds more \
             connector instances than the response can carry"
        );
        summaries.truncate(CONNECTOR_LIMIT as usize);
    }
    summaries
}

pub(super) fn absent_ledger(error: &clickhouse::error::Error) -> bool {
    let clickhouse::error::Error::BadResponse(payload) = error else {
        return false;
    };
    MissingRelation::from_message(payload).is_some()
}

#[cfg(test)]
mod guards {
    use super::*;

    const MIGRATION: &str = include_str!(
        "../../../../../../ingestion/scripts/migrations/20260827000000_connector-sync-history.sql"
    );

    fn statements() -> [(&'static str, &'static str); 6] {
        [
            ("SEALED_TICK_SQL", SEALED_TICK_SQL.as_str()),
            ("READ_INTERVAL_SQL", READ_INTERVAL_SQL.as_str()),
            ("LAST_SYNC_SQL", LAST_SYNC_SQL.as_str()),
            ("CONFIGURED_SET_SQL", CONFIGURED_SET_SQL.as_str()),
            ("SYNC_HISTORY_SQL", SYNC_HISTORY_SQL.as_str()),
            ("INSTANCE_HISTORY_SQL", INSTANCE_HISTORY_SQL.as_str()),
        ]
    }

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
        for expected in [
            "event_id",
            "ts",
            "tick_id",
            "job_id",
            "connector",
            "tenant_id",
            "source_id",
            "event",
            "schedules",
            "schedule_timezone",
            "schedule_origin",
            "schedule_suspended",
        ] {
            assert!(
                columns.iter().any(|c| c == expected),
                "missing {expected}: {columns:?}"
            );
        }
        assert_eq!(columns.len(), 17, "{columns:?}");
    }

    #[test]
    fn the_summary_groups_by_the_whole_identity() {
        let sql = normalised(&LAST_SYNC_SQL);
        assert!(
            sql.contains("group by connector, tenant_id, source_id"),
            "{sql}"
        );
        let configured = normalised(&CONFIGURED_SET_SQL);
        assert!(
            configured.contains("distinct connector, tenant_id, source_id"),
            "{configured}"
        );
    }

    #[test]
    fn a_scoped_window_filters_on_both_halves_of_the_identity() {
        let sql = normalised(&INSTANCE_HISTORY_SQL);
        assert!(sql.contains("tenant_id = ?"), "{sql}");
        assert!(sql.contains("source_id = ?"), "{sql}");
        assert!(
            !normalised(&SYNC_HISTORY_SQL).contains("tenant_id = ?"),
            "the unscoped window must span every instance under the name"
        );
    }

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
    fn the_summary_cannot_answer_with_more_instances_than_the_cap() {
        let configured = (0..CONNECTOR_LIMIT + 10)
            .map(|n| {
                (
                    InstanceKey {
                        connector: format!("connector-{n}"),
                        tenant_id: "example-tenant".to_owned(),
                        source_id: "main".to_owned(),
                    },
                    None,
                )
            })
            .collect();

        let summaries = merge(Vec::new(), configured);

        assert_eq!(summaries.len(), CONNECTOR_LIMIT as usize);
    }

    #[test]
    fn configured_siblings_keep_their_own_schedules_through_the_merge() {
        let configured = [
            ("main", "0 5 * * *", "UTC", "annotation", true),
            ("second", "0 17 * * *", "Europe/Berlin", "descriptor", false),
        ]
        .map(|(source, cron, timezone, origin, suspended)| {
            ConnectorRow {
                connector: "example-tracker".to_owned(),
                tenant_id: "example-tenant".to_owned(),
                source_id: source.to_owned(),
                schedules: vec![cron.to_owned()],
                schedule_timezone: timezone.to_owned(),
                schedule_origin: origin.to_owned(),
                schedule_suspended: suspended,
            }
            .into_entry()
        });
        let syncs = ["main", "second"].map(|source| SyncRow {
            connector: "example-tracker".to_owned(),
            tenant_id: "example-tenant".to_owned(),
            source_id: source.to_owned(),
            job_id: "1".to_owned(),
            status: "succeeded".to_owned(),
            started_at: None,
            job_updated_at: None,
            duration_ms: None,
            records_reported: None,
        });

        let summaries = merge(Vec::from(syncs), HashMap::from(configured));

        assert_eq!(summaries.len(), 2);
        for (source, expected) in [
            (
                "main",
                serde_json::json!({"crons": ["0 5 * * *"], "timezone": "UTC", "origin": "annotation", "suspended": true}),
            ),
            (
                "second",
                serde_json::json!({"crons": ["0 17 * * *"], "timezone": "Europe/Berlin", "origin": "descriptor", "suspended": false}),
            ),
        ] {
            let summary = summaries
                .iter()
                .find(|row| row.instance.source_id == source)
                .unwrap_or_else(|| panic!("missing {source}"));
            assert!(summary.configured, "{source}");
            let schedule = serde_json::to_value(&summary.schedule)
                .unwrap_or_else(|error| panic!("serialisable: {error}"));
            assert_eq!(schedule, expected, "{source}");
        }
    }

    #[test]
    fn only_the_configured_instance_carries_its_recorded_schedule() {
        let (key, schedule) = ConnectorRow {
            connector: "example-tracker".to_owned(),
            tenant_id: "example-tenant".to_owned(),
            source_id: "main".to_owned(),
            schedules: vec!["0 22 * * *".to_owned()],
            schedule_timezone: "Europe/Sofia".to_owned(),
            schedule_origin: "annotation".to_owned(),
            schedule_suspended: true,
        }
        .into_entry();
        let syncs = ["main", "removed"].map(|source| SyncRow {
            connector: key.connector.clone(),
            tenant_id: key.tenant_id.clone(),
            source_id: source.to_owned(),
            job_id: "1".to_owned(),
            status: "succeeded".to_owned(),
            started_at: None,
            job_updated_at: None,
            duration_ms: None,
            records_reported: None,
        });
        let summaries = merge(Vec::from(syncs), HashMap::from([(key, schedule)]));

        for summary in summaries {
            let json = serde_json::to_value(super::super::model::ConnectorHealth::from(summary))
                .unwrap_or_else(|error| panic!("serialisable: {error}"));
            if json["source_id"] == "main" {
                assert_eq!(json["schedule"]["crons"], serde_json::json!(["0 22 * * *"]));
                assert_eq!(json["schedule"]["timezone"], "Europe/Sofia");
                assert_eq!(json["schedule"]["origin"], "annotation");
                assert_eq!(json["schedule"]["suspended"], true);
            } else {
                assert_eq!(json["configured"], false);
                assert_eq!(json["schedule"], serde_json::Value::Null);
            }
        }
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

    #[test]
    fn the_configured_set_is_bounded_by_its_tick_not_by_a_limit() {
        let sql = normalised(&CONFIGURED_SET_SQL);
        assert!(sql.contains("tick_id = ?"), "{sql}");
        assert!(!sql.contains("limit "), "{sql}");
    }
}
