use chrono::{Duration, Utc};

use super::Error;

type R = Result<(), Box<dyn std::error::Error>>;

pub(in crate::domain) async fn configured_instances_follow_the_latest_completed_tick(
    ch: &insight_clickhouse::Client,
) -> R {
    ch.query("TRUNCATE TABLE ingestion_history.sync_events")
        .execute()
        .await?;

    let now = Utc::now();
    let older_ms = (now - Duration::minutes(30)).timestamp_millis();
    let configured_ms = (now - Duration::minutes(20)).timestamp_millis();
    let pass_completed_ms = (now - Duration::minutes(10)).timestamp_millis();
    let incomplete_pass_ms = (now - Duration::minutes(5)).timestamp_millis();

    ch.query(
        "INSERT INTO ingestion_history.sync_events \
         (ts, tick_id, connector, tenant_id, source_id, event, schedules, \
          schedule_timezone, schedule_origin, schedule_suspended) VALUES \
         (fromUnixTimestamp64Milli(?), 'tick-old', 'example-tracker', 'tenant-under-test', 'main', \
          'connector.configured', ['0 1 * * *'], 'UTC', 'descriptor', false), \
         (fromUnixTimestamp64Milli(?), 'tick-old', 'example-tracker', 'tenant-under-test', 'old-only', \
          'connector.configured', ['0 2 * * *'], 'UTC', 'descriptor', false), \
         (fromUnixTimestamp64Milli(?), 'tick-completed', 'example-tracker', 'tenant-under-test', 'main', \
          'connector.configured', ['0 5 * * *', '0 17 * * *'], 'UTC', 'api', true), \
         (fromUnixTimestamp64Milli(?), 'tick-completed', 'example-tracker', 'another-tenant', 'second', \
          'connector.configured', ['0 7 * * *'], '', 'annotation', false), \
         (fromUnixTimestamp64Milli(?), 'tick-open', 'example-tracker', 'tenant-under-test', 'main', \
          'connector.configured', ['0 23 * * *'], 'UTC', 'descriptor', false), \
         (fromUnixTimestamp64Milli(?), 'tick-open', 'example-tracker', 'tenant-under-test', 'open-only', \
          'connector.configured', ['0 23 * * *'], 'UTC', 'descriptor', false), \
         (fromUnixTimestamp64Milli(?), 'tick-completed', '', '', '', 'sweep.completed', [], '', '', false), \
         (fromUnixTimestamp64Milli(?), 'tick-old', '', '', '', 'sweep.completed', [], '', '', false)",
    )
    .bind(older_ms)
    .bind(older_ms)
    .bind(configured_ms)
    .bind(configured_ms)
    .bind(incomplete_pass_ms)
    .bind(incomplete_pass_ms)
    .bind(pass_completed_ms)
    .bind(older_ms)
    .execute()
    .await?;

    ch.query(
        "INSERT INTO ingestion_history.sync_events \
         (ts, tick_id, connector, tenant_id, source_id, event) VALUES \
         (fromUnixTimestamp64Milli(?), 'tick-completed', 'example-tracker', \
          'tenant-under-test', 'no-schedule', 'connector.configured')",
    )
    .bind(configured_ms)
    .execute()
    .await?;

    for source in ["main", "second", "no-schedule"] {
        let instance = super::Instance::parse("example-tracker", source)?;
        super::membership::require(ch, &instance).await?;
    }

    for source in ["old-only", "open-only"] {
        assert!(
            matches!(
                super::membership::require(ch, &super::Instance::parse("example-tracker", source)?)
                    .await,
                Err(Error::NotFound { .. })
            ),
            "{source} is absent from the latest completed tick"
        );
    }
    ch.query("TRUNCATE TABLE ingestion_history.sync_events")
        .execute()
        .await?;
    Ok(())
}
