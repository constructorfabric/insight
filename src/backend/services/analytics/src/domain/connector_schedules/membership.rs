use serde::Deserialize;

use super::{Error, Instance};

const CONFIGURED_INSTANCE_SQL: &str = "SELECT connector \
    FROM ingestion_history.sync_events \
    CROSS JOIN (SELECT tick_id AS completed_tick \
        FROM ingestion_history.sync_events WHERE event = 'sweep.completed' \
        ORDER BY ts DESC LIMIT 1) AS latest_completed_pass \
    WHERE event = 'connector.configured' AND tick_id = completed_tick \
        AND connector = ? AND source_id = ? \
    ORDER BY ts DESC LIMIT 1";

#[derive(Debug, Deserialize, clickhouse::Row)]
struct ConfiguredInstanceRecord {
    connector: String,
}

pub(super) async fn require(
    ch: &insight_clickhouse::Client,
    instance: &Instance,
) -> Result<(), Error> {
    let record = ch
        .query(CONFIGURED_INSTANCE_SQL)
        .bind(instance.connector.as_str())
        .bind(instance.source_id.as_str())
        .fetch_optional::<ConfiguredInstanceRecord>()
        .await?
        .ok_or_else(|| Error::NotFound {
            connector: instance.connector.as_str().to_owned(),
            source_id: instance.source_id.as_str().to_owned(),
        })?;
    drop(record.connector);
    Ok(())
}
