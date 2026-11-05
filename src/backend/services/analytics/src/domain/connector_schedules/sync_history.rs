use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::dto::RecordedSchedule;

const COMPLETED_PASSES_SQL: &str = "SELECT max(ts) AS pass_completed_at \
    FROM ingestion_history.sync_events WHERE event = 'sweep.completed' AND ts <= fromUnixTimestamp64Milli(?) \
    GROUP BY tick_id ORDER BY pass_completed_at DESC LIMIT 2";

#[derive(Debug, Deserialize, clickhouse::Row)]
struct CompletedPassRecord {
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    pass_completed_at: DateTime<Utc>,
}

pub(super) async fn completed_passes(
    ch: &insight_clickhouse::Client,
    through: DateTime<Utc>,
) -> Result<Vec<DateTime<Utc>>, super::Error> {
    Ok(ch
        .query(COMPLETED_PASSES_SQL)
        .bind(through.timestamp_millis())
        .fetch_all::<CompletedPassRecord>()
        .await?
        .into_iter()
        .map(|row| row.pass_completed_at)
        .collect())
}

const CONFIGURED_INSTANCE_SQL: &str = "SELECT schedules, schedule_timezone, schedule_origin, schedule_suspended, pass_completed_at \
    FROM ingestion_history.sync_events \
    CROSS JOIN (SELECT tick_id AS completed_tick, ts AS pass_completed_at \
        FROM ingestion_history.sync_events WHERE event = 'sweep.completed' \
        ORDER BY ts DESC LIMIT 1) AS latest_completed_pass \
    WHERE event = 'connector.configured' AND tick_id = completed_tick \
        AND connector = ? AND source_id = ? \
    ORDER BY ts DESC LIMIT 1";

#[derive(Debug, Deserialize, clickhouse::Row)]
pub(super) struct ConfiguredInstanceRecord {
    pub schedules: Vec<String>,
    pub schedule_timezone: String,
    pub schedule_origin: String,
    pub schedule_suspended: bool,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    pub pass_completed_at: DateTime<Utc>,
}

pub(super) async fn applied_schedule_and_pass_completion(
    ch: &insight_clickhouse::Client,
    connector: &str,
    source_id: &str,
) -> Result<(Option<RecordedSchedule>, DateTime<Utc>), super::Error> {
    let row = ch
        .query(CONFIGURED_INSTANCE_SQL)
        .bind(connector)
        .bind(source_id)
        .fetch_optional::<ConfiguredInstanceRecord>()
        .await?
        .ok_or_else(|| super::Error::NotFound {
            connector: connector.to_owned(),
            source_id: source_id.to_owned(),
        })?;
    Ok((
        (!row.schedules.is_empty()).then_some(RecordedSchedule {
            crons: row.schedules,
            timezone: (!row.schedule_timezone.is_empty()).then_some(row.schedule_timezone),
            origin: row.schedule_origin,
            suspended: row.schedule_suspended,
            recorded_at: row.pass_completed_at,
        }),
        row.pass_completed_at,
    ))
}

#[cfg(test)]
pub(super) async fn applied_schedule(
    ch: &insight_clickhouse::Client,
    connector: &str,
    source_id: &str,
) -> Result<Option<RecordedSchedule>, super::Error> {
    applied_schedule_and_pass_completion(ch, connector, source_id)
        .await
        .map(|(applied, _)| applied)
}
